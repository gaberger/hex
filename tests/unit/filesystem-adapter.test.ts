import { describe, it, expect, afterAll } from 'bun:test';
import { mkdtemp, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { FileSystemAdapter, PathTraversalError } from '../../src/adapters/secondary/filesystem-adapter.js';

let tempDir: string;
let fs: FileSystemAdapter;

// Setup: create a unique temp directory
const setup = async () => {
  tempDir = await mkdtemp(join(tmpdir(), 'hex-test-'));
  fs = new FileSystemAdapter(tempDir);
};

afterAll(async () => {
  if (tempDir) await rm(tempDir, { recursive: true, force: true });
});

describe('FileSystemAdapter', () => {
  it('write creates file and parent directories', async () => {
    await setup();
    await fs.write('deep/nested/dir/file.txt', 'hello world');
    const content = await fs.read('deep/nested/dir/file.txt');
    expect(content).toBe('hello world');
  });

  it('read returns file content', async () => {
    await setup();
    await fs.write('readme.txt', 'test content');
    const content = await fs.read('readme.txt');
    expect(content).toBe('test content');
  });

  it('exists returns true for existing file', async () => {
    await setup();
    await fs.write('exists.txt', 'yes');
    expect(await fs.exists('exists.txt')).toBe(true);
  });

  it('exists returns false for missing file', async () => {
    await setup();
    expect(await fs.exists('nope.txt')).toBe(false);
  });

  it('glob matches patterns correctly', async () => {
    await setup();
    await fs.write('src/a.ts', '');
    await fs.write('src/b.ts', '');
    await fs.write('src/c.js', '');
    const tsFiles = await fs.glob('src/*.ts');
    expect(tsFiles).toHaveLength(2);
    expect(tsFiles).toContain('src/a.ts');
    expect(tsFiles).toContain('src/b.ts');
  });

  it('blocks path traversal on read', async () => {
    await setup();
    expect(fs.read('../../../etc/passwd')).rejects.toThrow(PathTraversalError);
  });

  it('blocks path traversal on write', async () => {
    await setup();
    expect(fs.write('../../escape.txt', 'pwned')).rejects.toThrow(PathTraversalError);
  });

  it('blocks path traversal on exists', async () => {
    await setup();
    expect(fs.exists('../../../etc/shadow')).rejects.toThrow(PathTraversalError);
  });

  it('blocks path traversal in glob patterns', async () => {
    await setup();
    expect(fs.glob('../../**/*.ts')).rejects.toThrow(PathTraversalError);
  });
});
