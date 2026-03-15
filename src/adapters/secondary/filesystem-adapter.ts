/**
 * Filesystem secondary adapter -- implements IFileSystemPort.
 *
 * Uses Node/Bun fs APIs for file operations with path resolution
 * relative to a configurable root directory.
 */
import { access, mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import type { IFileSystemPort } from '../../core/ports/index.js';

export class PathTraversalError extends Error {
  constructor(filePath: string, root: string) {
    super(`Path traversal blocked: "${filePath}" resolves outside root "${root}"`);
    this.name = 'PathTraversalError';
  }
}

export class FileSystemAdapter implements IFileSystemPort {
  private readonly root: string;

  constructor(rootPath?: string) {
    this.root = rootPath ? resolve(rootPath) : process.cwd();
  }

  async read(filePath: string): Promise<string> {
    const abs = this.safePath(filePath);
    return readFile(abs, { encoding: 'utf-8' });
  }

  async write(filePath: string, content: string): Promise<void> {
    const abs = this.safePath(filePath);
    await mkdir(dirname(abs), { recursive: true });
    await writeFile(abs, content, { encoding: 'utf-8' });
  }

  async exists(filePath: string): Promise<boolean> {
    const abs = this.safePath(filePath); // throws PathTraversalError before try
    try {
      await access(abs);
      return true;
    } catch {
      return false;
    }
  }

  async glob(pattern: string): Promise<string[]> {
    // Glob patterns are constrained to root via cwd option
    if (pattern.includes('..')) {
      throw new PathTraversalError(pattern, this.root);
    }
    const g = new Bun.Glob(pattern);
    const matches: string[] = [];
    for await (const entry of g.scan({ cwd: this.root, absolute: false })) {
      matches.push(entry);
    }
    return matches.sort();
  }

  /** Resolve path and verify it stays within root — prevents directory traversal */
  private safePath(filePath: string): string {
    const abs = resolve(join(this.root, filePath));
    if (!abs.startsWith(this.root)) {
      throw new PathTraversalError(filePath, this.root);
    }
    return abs;
  }
}
