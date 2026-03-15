/**
 * Tree-sitter secondary adapter -- implements IASTPort.
 *
 * Uses web-tree-sitter (WASM) to parse TypeScript files and produce
 * L0-L3 AST summaries as defined in docs/architecture/treesitter-format.md.
 */
import { resolve as pathResolve } from 'node:path';
import type { Parser, Language as TSLanguage, Node as TSNode, Tree } from 'web-tree-sitter';
import type {
  ASTSummary,
  ExportEntry,
  IASTPort,
  IFileSystemPort,
  ImportEntry,
  Language,
  StructuralDiff,
} from '../../core/ports/index.js';

/** Maps tree-sitter node types to ExportEntry.kind values. */
const TS_NODE_KIND_MAP: Record<string, string> = {
  function_declaration: 'function',
  class_declaration: 'class',
  interface_declaration: 'interface',
  type_alias_declaration: 'type',
  enum_declaration: 'enum',
  lexical_declaration: 'const',
};

export class TreeSitterAdapter implements IASTPort {
  private parser: Parser | undefined;
  private langMap = new Map<Language, TSLanguage>();
  private _isStub = false;

  private constructor(
    private readonly grammarDirs: string[],
    private readonly fs: IFileSystemPort,
    private readonly rootPath: string,
  ) {}

  /**
   * Factory -- initialises web-tree-sitter and loads grammars from multiple
   * candidate directories (project-local config/grammars, tree-sitter-wasms
   * npm package, legacy web-tree-sitter directory).
   */
  static async create(grammarDirs: string | string[], fs: IFileSystemPort, rootPath?: string): Promise<TreeSitterAdapter> {
    const dirs = Array.isArray(grammarDirs) ? grammarDirs : [grammarDirs];
    const adapter = new TreeSitterAdapter(dirs, fs, rootPath ?? process.cwd());
    await adapter.init();
    return adapter;
  }

  /** Returns true if no grammars were loaded (all analysis returns empty). */
  isStub(): boolean {
    return this._isStub;
  }

  private async init(): Promise<void> {
    const mod = await import('web-tree-sitter');
    const ParserClass = mod.Parser;
    await ParserClass.init();
    this.parser = new ParserClass();

    const grammarFiles: [Language, string][] = [
      ['typescript', 'tree-sitter-typescript.wasm'],
      ['go', 'tree-sitter-go.wasm'],
      ['rust', 'tree-sitter-rust.wasm'],
    ];

    for (const [lang, filename] of grammarFiles) {
      const wasmPath = await this.findGrammar(filename);
      if (wasmPath) {
        const loaded = await mod.Language.load(wasmPath);
        this.langMap.set(lang, loaded);
      }
    }

    this._isStub = this.langMap.size === 0;
  }

  /**
   * Search candidate directories for a grammar WASM file.
   * Returns an ABSOLUTE path because Language.load() needs a real filesystem path,
   * but uses fs.exists() with relative paths for safe traversal checking.
   */
  private async findGrammar(filename: string): Promise<string | null> {
    for (const dir of this.grammarDirs) {
      const relative = `${dir}/${filename}`;
      if (await this.fs.exists(relative)) {
        return pathResolve(this.rootPath, relative);
      }
    }
    return null;
  }

  // ── IASTPort ──────────────────────────────────────────────

  async extractSummary(filePath: string, level: ASTSummary['level']): Promise<ASTSummary> {
    const source = await this.fs.read(filePath);
    const lang = detectLanguage(filePath);
    const lineCount = source.split('\n').length;
    const fullTokenEstimate = Math.ceil(source.length / 4);

    if (level === 'L0') {
      return {
        filePath, language: lang, level,
        exports: [], imports: [], dependencies: [],
        lineCount,
        tokenEstimate: Math.ceil((filePath.length + 20) / 4), // ~filename + metadata
      };
    }

    if (level === 'L3') {
      return {
        filePath, language: lang, level,
        exports: [], imports: [], dependencies: [],
        lineCount, tokenEstimate: fullTokenEstimate,
        raw: source,
      };
    }

    // L1 or L2 — parse and extract structural summary
    const tree = this.parse(source, lang);
    const base: ASTSummary = {
      filePath, language: lang, level,
      exports: [], imports: [], dependencies: [],
      lineCount, tokenEstimate: 0, // computed below
    };
    if (tree === null) {
      base.tokenEstimate = fullTokenEstimate;
      return base;
    }

    base.exports = this.extractExports(tree, level === 'L2');
    base.imports = this.extractImports(tree);
    base.dependencies = base.imports.filter((i) => !i.from.startsWith('.')).map((i) => i.from);

    // Token estimate based on serialized summary size, not raw source
    const summaryText = base.exports.map(e => `${e.kind} ${e.name}${e.signature ? ': ' + e.signature : ''}`).join('\n')
      + '\n' + base.imports.map(i => `import {${i.names.join(',')}} from '${i.from}'`).join('\n');
    base.tokenEstimate = Math.ceil(summaryText.length / 4);

    return base;
  }

  diffStructural(before: ASTSummary, after: ASTSummary): StructuralDiff {
    const toMap = (list: ExportEntry[]) => new Map(list.map((e) => [e.name, e]));
    const bMap = toMap(before.exports);
    const aMap = toMap(after.exports);
    const added: ExportEntry[] = [];
    const removed: ExportEntry[] = [];
    const modified: StructuralDiff['modified'] = [];

    for (const [name, entry] of aMap) {
      const prev = bMap.get(name);
      if (!prev) added.push(entry);
      else if (prev.kind !== entry.kind || prev.signature !== entry.signature)
        modified.push({ before: prev, after: entry });
    }
    for (const [, entry] of bMap) {
      if (!aMap.has(entry.name)) removed.push(entry);
    }
    return { added, removed, modified };
  }

  // ── Private helpers ───────────────────────────────────────

  private parse(source: string, lang: Language): Tree | null {
    if (!this.parser) return null;
    const grammar = this.langMap.get(lang);
    if (!grammar) return null;
    this.parser.setLanguage(grammar);
    return this.parser.parse(source);
  }

  private extractExports(tree: Tree, withSigs: boolean): ExportEntry[] {
    const results: ExportEntry[] = [];
    const root = tree.rootNode;
    for (let i = 0; i < root.childCount; i++) {
      const node = root.child(i)!;
      if (node.type !== 'export_statement') continue;

      // Handle re-exports: `export type { X, Y } from './foo.js'`
      // and `export { X, Y } from './foo.js'`
      const exportClause = node.namedChildren.find((c) => c.type === 'export_clause');
      if (exportClause) {
        for (let j = 0; j < exportClause.namedChildCount; j++) {
          const spec = exportClause.namedChild(j)!;
          if (spec.type === 'export_specifier') {
            const alias = spec.childForFieldName('alias');
            const name = spec.childForFieldName('name');
            const exportName = (alias ?? name)?.text;
            if (exportName) {
              results.push({ name: exportName, kind: 'type' });
            }
          }
        }
        continue;
      }

      const decl = node.namedChildren.find((c) => c.type !== 'comment');
      if (!decl) continue;
      const kind = TS_NODE_KIND_MAP[decl.type] as ExportEntry['kind'] | undefined;
      if (!kind) continue;
      const nameNode = decl.childForFieldName('name')
        ?? decl.namedChildren.find((c) => c.type === 'identifier' || c.type === 'type_identifier');
      if (!nameNode) continue;
      const entry: ExportEntry = { name: nameNode.text, kind };
      if (withSigs) {
        const body = decl.childForFieldName('body');
        entry.signature = body
          ? decl.text.slice(0, body.startIndex - decl.startIndex).trim()
          : decl.text.trim();
      }
      results.push(entry);
    }
    return results;
  }

  private extractImports(tree: Tree): ImportEntry[] {
    const results: ImportEntry[] = [];
    const root = tree.rootNode;
    for (let i = 0; i < root.childCount; i++) {
      const node = root.child(i)!;
      if (node.type !== 'import_statement') continue;
      const srcNode = node.childForFieldName('source');
      const from = srcNode ? srcNode.text.replace(/['"]/g, '') : '';
      if (!from) continue;
      const names: string[] = [];
      const clause = node.namedChildren.find((c) => c.type === 'import_clause');
      if (clause) collectNames(clause, names);
      results.push({ names, from });
    }
    return results;
  }
}

// ── Module-level helpers ──────────────────────────────────────

function detectLanguage(filePath: string): Language {
  if (filePath.endsWith('.ts') || filePath.endsWith('.tsx')) return 'typescript';
  if (filePath.endsWith('.go')) return 'go';
  if (filePath.endsWith('.rs')) return 'rust';
  return 'typescript';
}

function collectNames(node: TSNode, out: string[]): void {
  if (node.type === 'import_specifier') {
    const alias = node.childForFieldName('alias');
    const name = node.childForFieldName('name');
    out.push((alias ?? name)?.text ?? node.text);
    return;
  }
  if (node.type === 'namespace_import') {
    const id = node.namedChildren.find((c: TSNode) => c.type === 'identifier');
    if (id) out.push(`* as ${id.text}`);
    return;
  }
  if (node.type === 'identifier' && node.parent?.type !== 'namespace_import') {
    out.push(node.text);
  }
  for (let i = 0; i < node.namedChildCount; i++) {
    collectNames(node.namedChild(i)!, out);
  }
}
