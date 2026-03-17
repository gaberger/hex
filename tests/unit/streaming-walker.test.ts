import { describe, it, expect, beforeAll, afterAll } from 'bun:test';
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { FileSystemAdapter } from '../../src/adapters/secondary/filesystem-adapter.js';

describe('FileSystemAdapter.streamFiles', () => {
  let tempDir: string;
  let adapter: FileSystemAdapter;

  beforeAll(async () => {
    tempDir = await mkdtemp(join(tmpdir(), 'hex-stream-test-'));
    adapter = new FileSystemAdapter(tempDir);

    // Build a small directory tree:
    // root/
    //   a.ts
    //   b.js
    //   sub/
    //     c.ts
    //     deep/
    //       d.ts
    //       deeper/
    //         e.ts
    //   node_modules/
    //     pkg/
    //       index.js
    //   target/
    //     out.js
    //   empty/

    await mkdir(join(tempDir, 'sub', 'deep', 'deeper'), { recursive: true });
    await mkdir(join(tempDir, 'node_modules', 'pkg'), { recursive: true });
    await mkdir(join(tempDir, 'target'), { recursive: true });
    await mkdir(join(tempDir, 'empty'), { recursive: true });

    await writeFile(join(tempDir, 'a.ts'), 'export const a = 1;');
    await writeFile(join(tempDir, 'b.js'), 'module.exports = {};');
    await writeFile(join(tempDir, 'sub', 'c.ts'), 'export const c = 3;');
    await writeFile(join(tempDir, 'sub', 'deep', 'd.ts'), 'export const d = 4;');
    await writeFile(join(tempDir, 'sub', 'deep', 'deeper', 'e.ts'), 'export const e = 5;');
    await writeFile(join(tempDir, 'node_modules', 'pkg', 'index.js'), '// pkg');
    await writeFile(join(tempDir, 'target', 'out.js'), '// compiled');
  });

  afterAll(async () => {
    await rm(tempDir, { recursive: true, force: true });
  });

  it('yields files from a directory tree', async () => {
    const files: string[] = [];
    for await (const f of adapter.streamFiles('**/*')) {
      files.push(f);
    }
    // node_modules and .git are always excluded by the implementation
    expect(files).toContain('a.ts');
    expect(files).toContain('b.js');
    expect(files).toContain('sub/c.ts');
    expect(files).toContain('sub/deep/d.ts');
    expect(files).toContain('sub/deep/deeper/e.ts');
  });

  it('respects ignore patterns (skips node_modules/, target/)', async () => {
    const files: string[] = [];
    for await (const f of adapter.streamFiles('**/*', {
      ignore: ['target/'],
    })) {
      files.push(f);
    }
    // node_modules is always excluded; target/ excluded via ignore option
    const hasNodeModules = files.some((f) => f.includes('node_modules'));
    const hasTarget = files.some((f) => f.includes('target'));
    expect(hasNodeModules).toBe(false);
    expect(hasTarget).toBe(false);
  });

  it('respects maxDepth option', async () => {
    const files: string[] = [];
    for await (const f of adapter.streamFiles('**/*', { maxDepth: 1 })) {
      files.push(f);
    }
    // Depth 0 = root dir entries (a.ts, b.js)
    // Depth 1 = one level down (sub/c.ts)
    // Should NOT include depth 2+ (sub/deep/d.ts)
    expect(files).toContain('a.ts');
    expect(files).toContain('b.js');
    expect(files).toContain('sub/c.ts');
    expect(files).not.toContain('sub/deep/d.ts');
    expect(files).not.toContain('sub/deep/deeper/e.ts');
  });

  it('handles empty directories', async () => {
    // Create an adapter rooted at the empty directory
    const emptyAdapter = new FileSystemAdapter(join(tempDir, 'empty'));
    const files: string[] = [];
    for await (const f of emptyAdapter.streamFiles('**/*')) {
      files.push(f);
    }
    expect(files).toEqual([]);
  });

  it('handles non-existent root gracefully', async () => {
    const badAdapter = new FileSystemAdapter(tempDir);
    const files: string[] = [];
    // streamFiles starts from a prefix-derived path; if it doesn't exist
    // the BFS just skips it silently (readdir catch)
    for await (const f of badAdapter.streamFiles('nonexistent/**/*')) {
      files.push(f);
    }
    expect(files).toEqual([]);
  });

  it('returns an AsyncGenerator (does not accumulate all results in memory)', async () => {
    const gen = adapter.streamFiles('**/*');
    // Verify it's an async generator (has next/return/throw and Symbol.asyncIterator)
    expect(typeof gen.next).toBe('function');
    expect(typeof gen.return).toBe('function');
    expect(typeof gen.throw).toBe('function');
    expect(typeof gen[Symbol.asyncIterator]).toBe('function');

    // Consume one item at a time to verify lazy yielding
    const first = await gen.next();
    expect(first.done).toBe(false);
    expect(typeof first.value).toBe('string');

    // Clean up the generator
    await gen.return(undefined as never);
  });

  it('filters by extension when pattern includes one', async () => {
    const files: string[] = [];
    for await (const f of adapter.streamFiles('**/*.ts')) {
      files.push(f);
    }
    expect(files).toContain('a.ts');
    expect(files).toContain('sub/c.ts');
    expect(files).not.toContain('b.js');
  });
});
