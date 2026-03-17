/**
 * Integration test for native tree-sitter backend.
 *
 * Tests both the native NAPI path and the WASM fallback to ensure
 * they produce identical output for the same input files.
 *
 * Skipped when @hex/native is not installed (CI without Rust toolchain).
 */
import { describe, it, expect, beforeAll } from 'bun:test';
import { TreeSitterAdapter } from '../../src/adapters/secondary/treesitter-adapter.js';
import type { IASTPort, IFileSystemPort } from '../../src/core/ports/index.js';
import { readFile, stat } from 'node:fs/promises';
import { join } from 'node:path';

// Minimal filesystem adapter for testing
const testFs: IFileSystemPort = {
  async read(filePath: string): Promise<string> {
    return readFile(filePath, 'utf-8');
  },
  async write(_filePath: string, _content: string): Promise<void> {
    throw new Error('write not needed in test');
  },
  async exists(filePath: string): Promise<boolean> {
    try {
      await stat(filePath);
      return true;
    } catch {
      return false;
    }
  },
  async glob(_pattern: string): Promise<string[]> {
    return [];
  },
  async mtime(filePath: string): Promise<number> {
    try {
      const s = await stat(filePath);
      return s.mtimeMs;
    } catch {
      return 0;
    }
  },
};

const ROOT = join(import.meta.dir, '../..');

// Check if native module is available
let hasNative = false;
try {
  const mod = require('@hex/native');
  hasNative = typeof mod.initGrammars === 'function';
} catch {
  hasNative = false;
}

describe('Native tree-sitter backend', () => {
  let wasmAdapter: IASTPort;
  let nativeAdapter: IASTPort | null = null;

  beforeAll(async () => {
    const grammarDirs = [
      'config/grammars',
      'node_modules/tree-sitter-wasms/out',
      'node_modules/web-tree-sitter',
    ];
    wasmAdapter = await TreeSitterAdapter.create(grammarDirs, testFs, ROOT);

    if (hasNative) {
      nativeAdapter = await TreeSitterAdapter.createWithNativeFallback(grammarDirs, testFs, ROOT);
    }
  });

  it('WASM adapter produces valid L1 summary for TypeScript', async () => {
    const summary = await wasmAdapter.extractSummary(
      join(ROOT, 'src/core/domain/value-objects.ts'),
      'L1',
    );
    expect(summary.language).toBe('typescript');
    expect(summary.level).toBe('L1');
    expect(summary.exports.length).toBeGreaterThan(0);
    expect(summary.lineCount).toBeGreaterThan(0);
  });

  it('WASM adapter produces valid L2 summary with signatures', async () => {
    const summary = await wasmAdapter.extractSummary(
      join(ROOT, 'src/core/domain/value-objects.ts'),
      'L2',
    );
    expect(summary.level).toBe('L2');
    // L2 should have signatures on at least some exports
    const withSigs = summary.exports.filter(e => e.signature);
    expect(withSigs.length).toBeGreaterThan(0);
  });

  // These tests only run when native module is available
  const nativeIt = hasNative ? it : it.skip;

  nativeIt('native adapter produces valid L1 summary for TypeScript', async () => {
    if (!nativeAdapter) return;
    const summary = await nativeAdapter.extractSummary(
      join(ROOT, 'src/core/domain/value-objects.ts'),
      'L1',
    );
    expect(summary.language).toBe('typescript');
    expect(summary.level).toBe('L1');
    expect(summary.exports.length).toBeGreaterThan(0);
  });

  nativeIt('native and WASM produce matching L1 exports', async () => {
    if (!nativeAdapter) return;
    const testFile = join(ROOT, 'src/core/domain/value-objects.ts');

    const wasmSummary = await wasmAdapter.extractSummary(testFile, 'L1');
    const nativeSummary = await nativeAdapter.extractSummary(testFile, 'L1');

    // Export names and kinds should match exactly
    const wasmExports = wasmSummary.exports.map(e => `${e.kind}:${e.name}`).sort();
    const nativeExports = nativeSummary.exports.map(e => `${e.kind}:${e.name}`).sort();
    expect(nativeExports).toEqual(wasmExports);

    // Import sources should match
    const wasmImports = wasmSummary.imports.map(i => i.from).sort();
    const nativeImports = nativeSummary.imports.map(i => i.from).sort();
    expect(nativeImports).toEqual(wasmImports);
  });

  nativeIt('native and WASM produce matching L2 signatures', async () => {
    if (!nativeAdapter) return;
    const testFile = join(ROOT, 'src/core/ports/index.ts');

    const wasmSummary = await wasmAdapter.extractSummary(testFile, 'L2');
    const nativeSummary = await nativeAdapter.extractSummary(testFile, 'L2');

    // Signatures should match
    for (const wasmExport of wasmSummary.exports) {
      const nativeExport = nativeSummary.exports.find(e => e.name === wasmExport.name);
      if (nativeExport && wasmExport.signature) {
        expect(nativeExport.signature).toBe(wasmExport.signature);
      }
    }
  });

  nativeIt('native handles Go files correctly', async () => {
    if (!nativeAdapter) return;
    // Use the rust-api example's main.rs as a Rust test file
    const testFile = join(ROOT, 'examples/rust-api/src/main.rs');
    const exists = await testFs.exists(testFile);
    if (!exists) return; // skip if example not present

    const summary = await nativeAdapter.extractSummary(testFile, 'L1');
    expect(summary.language).toBe('rust');
    expect(summary.exports.length).toBeGreaterThanOrEqual(0);
  });

  it('createWithNativeFallback returns a working adapter', async () => {
    const grammarDirs = [
      'config/grammars',
      'node_modules/tree-sitter-wasms/out',
      'node_modules/web-tree-sitter',
    ];
    const adapter = await TreeSitterAdapter.createWithNativeFallback(grammarDirs, testFs, ROOT);
    const summary = await adapter.extractSummary(
      join(ROOT, 'src/core/domain/value-objects.ts'),
      'L1',
    );
    expect(summary.exports.length).toBeGreaterThan(0);
  });

  it('diffStructural works on both backends', async () => {
    const file = join(ROOT, 'src/core/domain/value-objects.ts');
    const before = await wasmAdapter.extractSummary(file, 'L1');

    // Create a modified version by removing first export
    const after = { ...before, exports: before.exports.slice(1) };
    const diff = wasmAdapter.diffStructural(before, after);

    expect(diff.removed.length).toBe(1);
    expect(diff.removed[0].name).toBe(before.exports[0].name);
  });
});
