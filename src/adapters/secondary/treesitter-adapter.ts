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

// ── Native NAPI-RS backend (optional) ─────────────────────

type NativeModule = {
  initGrammars(): boolean;
  parseFile(filePath: string, source: string, level: 'L0' | 'L1' | 'L2' | 'L3'): ASTSummary;
};

let nativeModule: NativeModule | null = null;
let nativeProbed = false;

function tryLoadNative(): NativeModule | null {
  if (nativeProbed) return nativeModule;
  nativeProbed = true;
  try {
    // Dynamic require — only works if @hex/native is installed
    // eslint-disable-next-line @typescript-eslint/no-require-imports
    const mod = require('@hex/native');
    if (typeof mod.initGrammars === 'function' && mod.initGrammars()) {
      nativeModule = mod as NativeModule;
      return nativeModule;
    }
    return null;
  } catch {
    return null;
  }
}

/** Maps tree-sitter node types to ExportEntry.kind values per language. */
const TS_NODE_KIND_MAP: Record<string, string> = {
  function_declaration: 'function',
  class_declaration: 'class',
  interface_declaration: 'interface',
  type_alias_declaration: 'type',
  enum_declaration: 'enum',
  lexical_declaration: 'const',
};

const GO_NODE_KIND_MAP: Record<string, string> = {
  function_declaration: 'function',
  method_declaration: 'function',
  type_declaration: 'type', // covers struct, interface, type alias
  const_declaration: 'const',
  var_declaration: 'const',
};

const RUST_NODE_KIND_MAP: Record<string, string> = {
  function_item: 'function',
  struct_item: 'type',
  trait_item: 'interface',
  enum_item: 'enum',
  type_item: 'type',
  const_item: 'const',
  static_item: 'const',
  impl_item: 'type',
};

export class TreeSitterAdapter implements IASTPort {
  private parser: Parser | undefined;
  private langMap = new Map<Language, TSLanguage>();
  /** Cache: filePath:level → { mtime, summary }. Avoids re-parsing unchanged files. */
  private summaryCache = new Map<string, { mtime: number; summary: ASTSummary }>();
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
    // Check mtime cache — skip re-parsing unchanged files
    const cacheKey = `${filePath}:${level}`;
    const mtime = await this.fs.mtime(filePath).catch(() => 0);
    const cached = this.summaryCache.get(cacheKey);
    if (cached && cached.mtime === mtime && mtime > 0) {
      return cached.summary;
    }

    const source = await this.fs.read(filePath);
    const lang = detectLanguage(filePath);
    const lineCount = source.split('\n').length;
    const fullTokenEstimate = Math.ceil(source.length / 4);

    // When tree-sitter has no grammars, return a clearly-stubbed summary
    // with empty analysis results so callers know data is not real.
    if (this._isStub) {
      const stub = {
        filePath, language: lang, level,
        exports: [], imports: [], dependencies: [],
        lineCount, tokenEstimate: fullTokenEstimate,
        stubbed: true,
        ...(level === 'L3' ? { raw: source } : {}),
      };
      if (mtime > 0) this.summaryCache.set(cacheKey, { mtime, summary: stub });
      return stub;
    }

    if (level === 'L0') {
      const l0 = {
        filePath, language: lang, level,
        exports: [], imports: [], dependencies: [],
        lineCount,
        tokenEstimate: Math.ceil((filePath.length + 20) / 4), // ~filename + metadata
      };
      if (mtime > 0) this.summaryCache.set(cacheKey, { mtime, summary: l0 });
      return l0;
    }

    if (level === 'L3') {
      const l3 = {
        filePath, language: lang, level,
        exports: [], imports: [], dependencies: [],
        lineCount, tokenEstimate: fullTokenEstimate,
        raw: source,
      };
      // Don't cache L3 — it contains the full source string (memory heavy)
      return l3;
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

    base.exports = this.extractExports(tree, level === 'L2', lang);
    base.imports = this.extractImports(tree, lang);
    base.dependencies = base.imports.filter((i) => !i.from.startsWith('.')).map((i) => i.from);

    // Token estimate based on serialized summary size, not raw source
    const summaryText = base.exports.map(e => `${e.kind} ${e.name}${e.signature ? ': ' + e.signature : ''}`).join('\n')
      + '\n' + base.imports.map(i => `import {${i.names.join(',')}} from '${i.from}'`).join('\n');
    base.tokenEstimate = Math.ceil(summaryText.length / 4);

    if (mtime > 0) this.summaryCache.set(cacheKey, { mtime, summary: base });
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

  private extractExports(tree: Tree, withSigs: boolean, lang: Language): ExportEntry[] {
    if (lang === 'go') return this.extractGoExports(tree, withSigs);
    if (lang === 'rust') return this.extractRustExports(tree, withSigs);
    return this.extractTsExports(tree, withSigs);
  }

  private extractTsExports(tree: Tree, withSigs: boolean): ExportEntry[] {
    const results: ExportEntry[] = [];
    const root = tree.rootNode;
    for (let i = 0; i < root.childCount; i++) {
      const node = root.child(i)!;
      if (node.type !== 'export_statement') continue;

      // Extract @hex: annotations from preceding comment (Layer 3 of dead-export fix)
      const annotations = extractHexAnnotations(root, i);

      // Handle re-exports: `export type { X, Y } from './foo.js'`
      const exportClause = node.namedChildren.find((c) => c != null && c.type === 'export_clause');
      if (exportClause) {
        for (let j = 0; j < exportClause.namedChildCount; j++) {
          const spec = exportClause.namedChild(j)!;
          if (spec.type === 'export_specifier') {
            const alias = spec.childForFieldName('alias');
            const name = spec.childForFieldName('name');
            const exportName = (alias ?? name)?.text;
            if (exportName) {
              results.push({ name: exportName, kind: 'type', ...(annotations.length > 0 ? { annotations } : {}) });
            }
          }
        }
        continue;
      }

      // Check for default export: `export default class Foo {}`, `export default function bar()`, `export default expr`
      const hasDefault = node.children.some((c) => c != null && c.type === 'default');
      if (hasDefault) {
        const decl = node.namedChildren.find((c) => c != null && c.type !== 'comment');
        if (!decl) continue;
        const kind = TS_NODE_KIND_MAP[decl.type] as ExportEntry['kind'] | undefined;
        const nameNode = decl.childForFieldName('name')
          ?? decl.namedChildren.find((c) => c != null && (c.type === 'identifier' || c.type === 'type_identifier'));
        const exportName = nameNode?.text ?? 'default';
        const entry: ExportEntry = { name: exportName, kind: kind ?? 'const', ...(annotations.length > 0 ? { annotations } : {}) };
        if (withSigs) {
          if (kind) {
            const body = decl.childForFieldName('body');
            entry.signature = body
              ? decl.text.slice(0, body.startIndex - decl.startIndex).trim()
              : decl.text.trim();
          } else {
            entry.signature = `default ${decl.text.trim()}`;
          }
        }
        results.push(entry);
        continue;
      }

      const decl = node.namedChildren.find((c) => c != null && c.type !== 'comment');
      if (!decl) continue;
      const kind = TS_NODE_KIND_MAP[decl.type] as ExportEntry['kind'] | undefined;
      if (!kind) continue;
      const nameNode = decl.childForFieldName('name')
        ?? decl.namedChildren.find((c) => c != null && (c.type === 'identifier' || c.type === 'type_identifier'));
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

  /**
   * Go exports: any top-level declaration with a capitalized name is public.
   * Handles function_declaration, method_declaration, type_declaration (struct/interface),
   * const_declaration, var_declaration.
   */
  private extractGoExports(tree: Tree, withSigs: boolean): ExportEntry[] {
    const results: ExportEntry[] = [];
    const root = tree.rootNode;

    for (let i = 0; i < root.childCount; i++) {
      const node = root.child(i)!;
      const kindStr = GO_NODE_KIND_MAP[node.type];
      if (!kindStr) continue;

      if (node.type === 'type_declaration') {
        // type_declaration contains one or more type_spec children
        for (let j = 0; j < node.namedChildCount; j++) {
          const typeSpec = node.namedChild(j)!;
          if (typeSpec.type !== 'type_spec') continue;
          const nameNode = typeSpec.childForFieldName('name');
          if (!nameNode || !isCapitalized(nameNode.text)) continue;
          // Determine if struct or interface
          const typeBody = typeSpec.childForFieldName('type');
          let kind: ExportEntry['kind'] = 'type';
          if (typeBody?.type === 'struct_type') kind = 'type';
          else if (typeBody?.type === 'interface_type') kind = 'interface';
          const entry: ExportEntry = { name: nameNode.text, kind };
          if (withSigs) entry.signature = typeSpec.text.split('{')[0]?.trim() ?? typeSpec.text;
          results.push(entry);
        }
        continue;
      }

      if (node.type === 'const_declaration' || node.type === 'var_declaration') {
        // May contain multiple specs
        for (let j = 0; j < node.namedChildCount; j++) {
          const spec = node.namedChild(j)!;
          const nameNode = spec.namedChildren.find((c) => c != null && c.type === 'identifier');
          if (!nameNode || !isCapitalized(nameNode.text)) continue;
          results.push({ name: nameNode.text, kind: 'const' });
        }
        continue;
      }

      // function_declaration or method_declaration
      const nameNode = node.childForFieldName('name');
      if (!nameNode) continue;
      const name = nameNode.text;
      if (!isCapitalized(name)) continue;

      const entry: ExportEntry = { name, kind: kindStr as ExportEntry['kind'] };
      if (withSigs) {
        const body = node.childForFieldName('body');
        entry.signature = body
          ? node.text.slice(0, body.startIndex - node.startIndex).trim()
          : node.text.trim();
      }
      results.push(entry);
    }
    return results;
  }

  /**
   * Rust exports: items with `pub` visibility modifier.
   * Handles function_item, struct_item, trait_item, enum_item,
   * type_item, const_item, static_item, impl_item.
   */
  private extractRustExports(tree: Tree, withSigs: boolean): ExportEntry[] {
    const results: ExportEntry[] = [];
    const root = tree.rootNode;

    for (let i = 0; i < root.childCount; i++) {
      const node = root.child(i)!;
      const kindStr = RUST_NODE_KIND_MAP[node.type];
      if (!kindStr) continue;

      // Check for pub visibility — only truly public items (not pub(crate) or pub(super))
      const visNode = node.namedChildren.find(
        (c) => c != null && c.type === 'visibility_modifier',
      );
      if (!visNode) continue;
      const visText = visNode.text.trim();
      const isRestrictedVis = visText === 'pub(crate)' || visText === 'pub(super)';
      if (isRestrictedVis) continue;

      // impl blocks: extract the type name being implemented, including trait impls
      if (node.type === 'impl_item') {
        const traitNode = node.childForFieldName('trait');
        const typeNode = node.childForFieldName('type');
        if (typeNode) {
          const implName = traitNode
            ? `impl ${traitNode.text} for ${typeNode.text}`
            : `impl ${typeNode.text}`;
          const entry: ExportEntry = { name: implName, kind: 'type' };
          if (withSigs) {
            const body = node.childForFieldName('body');
            entry.signature = body
              ? node.text.slice(0, body.startIndex - node.startIndex).trim()
              : node.text.trim();
          }
          results.push(entry);
        }
        continue;
      }

      const nameNode = node.childForFieldName('name');
      if (!nameNode) continue;

      const entry: ExportEntry = { name: nameNode.text, kind: kindStr as ExportEntry['kind'] };
      if (withSigs) {
        const body = node.childForFieldName('body');
        entry.signature = body
          ? node.text.slice(0, body.startIndex - node.startIndex).trim()
          : node.text.trim();
      }
      results.push(entry);
    }
    return results;
  }

  private extractImports(tree: Tree, lang: Language): ImportEntry[] {
    if (lang === 'go') return this.extractGoImports(tree);
    if (lang === 'rust') return this.extractRustImports(tree);
    return this.extractTsImports(tree);
  }

  private extractTsImports(tree: Tree): ImportEntry[] {
    const results: ImportEntry[] = [];
    const root = tree.rootNode;

    // Pass 1: Static imports (import { Foo } from './bar.js')
    for (let i = 0; i < root.childCount; i++) {
      const node = root.child(i)!;
      if (node.type !== 'import_statement') continue;
      const srcNode = node.childForFieldName('source');
      const from = srcNode ? srcNode.text.replace(/['"]/g, '') : '';
      if (!from) continue;
      const names: string[] = [];
      const clause = node.namedChildren.find((c) => c != null && c.type === 'import_clause');
      if (clause) collectNames(clause, names);
      results.push({ names, from });
    }

    // Pass 2: Re-exports (export { Foo } from './bar.js')
    // These are export_statement nodes with a source field.
    // Track them as imports so the dead-export analyzer can follow re-export chains.
    for (let i = 0; i < root.childCount; i++) {
      const node = root.child(i)!;
      if (node.type !== 'export_statement') continue;
      const srcNode = node.childForFieldName('source');
      if (!srcNode) continue; // Not a re-export
      const from = srcNode.text.replace(/['"]/g, '');
      if (!from) continue;

      const names: string[] = [];
      const exportClause = node.namedChildren.find((c) => c != null && c.type === 'export_clause');
      if (exportClause) {
        // export { X, Y } from './foo.js'
        collectNames(exportClause, names);
      } else {
        // export * from './foo.js' or export * as namespace from './foo.js'
        names.push('*');
      }
      results.push({ names, from });
    }

    // Pass 3: Dynamic imports — const { Foo } = await import('./bar.js')
    // These appear as call_expression nodes with `import` as the function.
    // Critical for composition-root patterns where adapters are loaded lazily.
    collectDynamicImports(root, results);

    return results;
  }

  /**
   * Go imports: `import "fmt"` or `import ( "fmt"; "net/http" )`.
   * Node type is `import_declaration` containing `import_spec` children.
   */
  private extractGoImports(tree: Tree): ImportEntry[] {
    const results: ImportEntry[] = [];
    const root = tree.rootNode;

    for (let i = 0; i < root.childCount; i++) {
      const node = root.child(i)!;
      if (node.type !== 'import_declaration') continue;

      const specs = collectGoImportSpecs(node);
      for (const spec of specs) {
        results.push(spec);
      }
    }
    return results;
  }

  /**
   * Rust imports: `use std::collections::HashMap;` or `use crate::core::ports::*;`.
   * Node type is `use_declaration`.
   */
  private extractRustImports(tree: Tree): ImportEntry[] {
    const results: ImportEntry[] = [];
    const root = tree.rootNode;

    for (let i = 0; i < root.childCount; i++) {
      const node = root.child(i)!;
      if (node.type !== 'use_declaration') continue;

      // The argument node contains the full path
      const arg = node.namedChildren.find(
        (c) => c != null && c.type !== 'visibility_modifier',
      );
      if (!arg) continue;

      const fullPath = arg.text.replace(/;$/, '').trim();
      // Extract the base module path (everything before ::{ or ::*)
      const basePath = fullPath.replace(/::\{.*\}$/, '').replace(/::\*$/, '');
      const names: string[] = [];

      // Extract named imports from `use foo::{Bar, Baz}`
      const braceMatch = fullPath.match(/::\{(.+)\}$/);
      if (braceMatch) {
        names.push(...braceMatch[1].split(',').map((n) => n.trim()));
      } else {
        // Single import: last segment is the name
        const segments = fullPath.split('::');
        names.push(segments[segments.length - 1]);
      }

      results.push({ names, from: basePath });
    }

    // Also capture `mod foo;` declarations (external submodule references, not inline mod blocks)
    for (let i = 0; i < root.childCount; i++) {
      const node = root.child(i)!;
      if (node.type !== 'mod_item') continue;
      // Inline modules have a `body` (declaration_list); skip those
      const body = node.childForFieldName('body');
      if (body) continue;
      const nameNode = node.childForFieldName('name');
      if (!nameNode) continue;
      const modName = nameNode.text;
      results.push({ names: [modName], from: `self::${modName}` });
    }

    return results;
  }
  /**
   * Factory -- tries native Rust/NAPI-RS backend first, falls back to WASM.
   *
   * Usage:
   *   const ast = await TreeSitterAdapter.createWithNativeFallback(grammarDirs, fs, rootPath);
   */
  static async createWithNativeFallback(
    grammarDirs: string | string[],
    fs: IFileSystemPort,
    rootPath?: string,
  ): Promise<IASTPort> {
    // Try native first
    const native = tryLoadNative();
    if (native) {
      process.stderr.write('[hex] Using native tree-sitter backend (Rust/NAPI-RS)\n');
      return NativeTreeSitterAdapter.create(native, fs);
    }

    // Fall back to WASM
    process.stderr.write('[hex] Using WASM tree-sitter backend\n');
    return TreeSitterAdapter.create(grammarDirs, fs, rootPath);
  }
}

/**
 * Native tree-sitter adapter — delegates parsing to the @hex/native NAPI-RS module.
 *
 * Same IASTPort interface, same mtime-based caching, but parsing runs through
 * compiled Rust instead of WASM. Instantiated only via TreeSitterAdapter.createWithNativeFallback().
 */
export class NativeTreeSitterAdapter implements IASTPort {
  private summaryCache = new Map<string, { mtime: number; summary: ASTSummary }>();

  private constructor(
    private readonly native: NativeModule,
    private readonly fs: IFileSystemPort,
  ) {}

  static create(native: NativeModule, fs: IFileSystemPort): NativeTreeSitterAdapter {
    return new NativeTreeSitterAdapter(native, fs);
  }

  /** Native module is fully functional — never a stub. */
  isStub(): boolean {
    return false;
  }

  async extractSummary(filePath: string, level: ASTSummary['level']): Promise<ASTSummary> {
    // Same mtime caching logic as WASM adapter
    const cacheKey = `${filePath}:${level}`;
    const mtime = await this.fs.mtime(filePath).catch(() => 0);
    const cached = this.summaryCache.get(cacheKey);
    if (cached && cached.mtime === mtime && mtime > 0) {
      return cached.summary;
    }

    const source = await this.fs.read(filePath);
    const summary = this.native.parseFile(filePath, source, level);

    // Don't cache L3 (full source, memory heavy)
    if (level !== 'L3' && mtime > 0) {
      this.summaryCache.set(cacheKey, { mtime, summary });
    }
    return summary;
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
}

// ── Module-level helpers ──────────────────────────────────────

function detectLanguage(filePath: string): Language {
  if (filePath.endsWith('.ts') || filePath.endsWith('.tsx')) return 'typescript';
  if (filePath.endsWith('.go')) return 'go';
  if (filePath.endsWith('.rs')) return 'rust';
  return 'typescript';
}

/** Go: capitalized names are exported. */
function isCapitalized(name: string): boolean {
  return name.length > 0 && name[0] >= 'A' && name[0] <= 'Z';
}

/** Collect import specs from a Go import_declaration node. */
function collectGoImportSpecs(node: TSNode): ImportEntry[] {
  const results: ImportEntry[] = [];

  for (let i = 0; i < node.namedChildCount; i++) {
    const child = node.namedChild(i)!;

    if (child.type === 'import_spec') {
      const pathNode = child.childForFieldName('path');
      const from = pathNode ? pathNode.text.replace(/"/g, '') : '';
      if (!from) continue;
      // The imported name is the last segment of the package path
      const segments = from.split('/');
      const alias = child.childForFieldName('name');
      // Handle blank imports (`_ "pkg"`) and dot imports (`. "pkg"`)
      const name = alias ? alias.text : segments[segments.length - 1];
      results.push({ names: [name], from });
    } else if (child.type === 'import_spec_list') {
      // Grouped imports: import ( "fmt"; "net/http" )
      for (let j = 0; j < child.namedChildCount; j++) {
        const spec = child.namedChild(j)!;
        if (spec.type !== 'import_spec') continue;
        const pathNode = spec.childForFieldName('path');
        const from = pathNode ? pathNode.text.replace(/"/g, '') : '';
        if (!from) continue;
        const segments = from.split('/');
        const alias = spec.childForFieldName('name');
        // Handle blank imports (`_ "pkg"`) and dot imports (`. "pkg"`)
        const name = alias ? alias.text : segments[segments.length - 1];
        results.push({ names: [name], from });
      }
    }
  }

  return results;
}

function collectNames(node: TSNode, out: string[]): void {
  if (node.type === 'import_specifier') {
    const alias = node.childForFieldName('alias');
    const name = node.childForFieldName('name');
    out.push((alias ?? name)?.text ?? node.text);
    return;
  }
  if (node.type === 'namespace_import') {
    const id = node.namedChildren.find((c): c is TSNode => c != null && c.type === 'identifier');
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

/**
 * Extract @hex: annotations from a comment preceding an export_statement.
 *
 * Checks the previous sibling in the root node's children. If it's a comment
 * containing `@hex:` tags (e.g., `@hex:public`, `@hex:dynamic`, `@hex:entry`),
 * returns the tag names as an array.
 */
function extractHexAnnotations(root: TSNode, exportIndex: number): string[] {
  if (exportIndex <= 0) return [];

  // Check the immediately preceding sibling
  const prev = root.child(exportIndex - 1);
  if (!prev || prev.type !== 'comment') return [];

  const text = prev.text;
  const matches = text.matchAll(/@hex:(\w+)/g);
  const tags: string[] = [];
  for (const m of matches) {
    tags.push(`hex:${m[1]}`);
  }
  return tags;
}

/**
 * Walk the AST to find dynamic `import()` calls and extract destructured names.
 *
 * Handles two patterns:
 *   const { Foo } = await import('./bar.js');         // variable_declaration
 *   const { Foo } = await import('./bar.js');         // nested in lexical_declaration
 *
 * The tree-sitter node structure for `const { Foo } = await import('./bar.js')`:
 *   lexical_declaration
 *     variable_declarator
 *       object_pattern → { Foo }
 *       await_expression
 *         call_expression
 *           import → keyword
 *           arguments → string literal
 */
function collectDynamicImports(node: TSNode, out: ImportEntry[]): void {
  for (let i = 0; i < node.childCount; i++) {
    const child = node.child(i)!;

    // Look for call_expression where the function is `import`
    if (child.type === 'call_expression') {
      const fn = child.child(0);
      if (fn && fn.type === 'import') {
        const args = child.childForFieldName('arguments');
        const firstArg = args?.namedChildren.find((c): c is TSNode => c != null && c.type === 'string');
        const from = firstArg?.text.replace(/['"]/g, '') ?? '';
        if (from) {
          // Walk up to find destructured names: const { Foo, Bar } = await import(...)
          const names = extractDestructuredNames(child);
          out.push({ names, from });
        }
        continue; // Don't recurse into this call_expression
      }
    }

    // Recurse into block-level statements (function bodies, if blocks, etc.)
    if (child.namedChildCount > 0) {
      collectDynamicImports(child, out);
    }
  }
}

/**
 * Given a `call_expression` for `import(...)`, walk up the AST to find
 * the destructuring pattern: `const { Foo } = await import(...)`.
 *
 * Returns extracted names or ['*'] if no destructuring found (namespace import).
 */
function extractDestructuredNames(importCall: TSNode): string[] {
  // Walk up: import() → await_expression → variable_declarator
  let current: TSNode | null = importCall;
  for (let depth = 0; depth < 4 && current; depth++) {
    current = current.parent;
    if (!current) break;

    if (current.type === 'variable_declarator') {
      const pattern = current.childForFieldName('name');
      if (pattern && pattern.type === 'object_pattern') {
        const names: string[] = [];
        for (let j = 0; j < pattern.namedChildCount; j++) {
          const prop = pattern.namedChild(j)!;
          if (prop.type === 'shorthand_property_identifier_pattern' || prop.type === 'shorthand_property_identifier') {
            names.push(prop.text);
          } else if (prop.type === 'pair_pattern') {
            const value = prop.childForFieldName('value');
            if (value) names.push(value.text);
          }
        }
        if (names.length > 0) return names;
      }
      // Non-destructured: const mod = await import(...) → treat as namespace
      return ['*'];
    }
  }
  // No destructuring found (bare `await import(...)` without assignment)
  return ['*'];
}
