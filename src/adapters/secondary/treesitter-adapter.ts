/**
 * Tree-sitter secondary adapter -- implements IASTPort.
 *
 * Uses web-tree-sitter (WASM) to parse TypeScript files and produce
 * L0-L3 AST summaries as defined in docs/architecture/treesitter-format.md.
 */
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
import { TS_NODE_KIND_MAP } from '../../infrastructure/treesitter/queries.js';

export class TreeSitterAdapter implements IASTPort {
  private parser: Parser | undefined;
  private langMap = new Map<Language, TSLanguage>();

  private constructor(
    private readonly grammarDir: string,
    private readonly fs: IFileSystemPort,
  ) {}

  /** Factory -- initialises web-tree-sitter and loads the TS grammar. */
  static async create(grammarDir: string, fs: IFileSystemPort): Promise<TreeSitterAdapter> {
    const adapter = new TreeSitterAdapter(grammarDir, fs);
    await adapter.init();
    return adapter;
  }

  private async init(): Promise<void> {
    const mod = await import('web-tree-sitter');
    const ParserClass = mod.Parser;
    await ParserClass.init();
    this.parser = new ParserClass();
    const wasmPath = `${this.grammarDir}/tree-sitter-typescript.wasm`;
    if (await this.fs.exists(wasmPath)) {
      const tsLang = await mod.Language.load(wasmPath);
      this.langMap.set('typescript', tsLang);
    }
  }

  // ── IASTPort ──────────────────────────────────────────────

  async extractSummary(filePath: string, level: ASTSummary['level']): Promise<ASTSummary> {
    const source = await this.fs.read(filePath);
    const lang = detectLanguage(filePath);
    const base: ASTSummary = {
      filePath, language: lang, level,
      exports: [], imports: [], dependencies: [],
      lineCount: source.split('\n').length,
      tokenEstimate: Math.ceil(source.length / 4),
    };
    if (level === 'L0') return base;
    if (level === 'L3') return { ...base, raw: source };

    const tree = this.parse(source, lang);
    if (tree === null) return base;

    base.exports = this.extractExports(tree, level === 'L2');
    base.imports = this.extractImports(tree);
    base.dependencies = base.imports.filter((i) => !i.from.startsWith('.')).map((i) => i.from);
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
