/**
 * Filesystem secondary adapter -- implements IFileSystemPort.
 *
 * Uses Node/Bun fs APIs for file operations with path resolution
 * relative to a configurable root directory.
 */
import { access, mkdir, readFile, writeFile } from 'node:fs/promises';
import { realpathSync } from 'node:fs';
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
    const resolved = rootPath ? resolve(rootPath) : process.cwd();
    // Resolve symlinks in the root itself so all comparisons use real paths
    this.root = realpathSync.native(resolved);
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

    // Use Bun.Glob when available, fall back to node:fs/promises glob
    if (typeof globalThis.Bun !== 'undefined') {
      const g = new Bun.Glob(pattern);
      const matches: string[] = [];
      for await (const entry of g.scan({ cwd: this.root, absolute: false })) {
        matches.push(entry);
      }
      return matches.sort();
    }

    // Node.js fallback using fs.glob (Node 22+) or recursive readdir
    try {
      const { glob: fsGlob } = await import('node:fs/promises');
      const matches: string[] = [];
      for await (const entry of fsGlob(pattern, { cwd: this.root })) {
        matches.push(entry);
      }
      return matches.sort();
    } catch {
      // Node < 22: manual recursive scan with extension matching
      return this.manualGlob(pattern);
    }
  }

  /** Fallback glob for Node versions without fs.glob */
  private async manualGlob(pattern: string): Promise<string[]> {
    const { readdir } = await import('node:fs/promises');
    // Extract extension from pattern like "**/*.ts" → ".ts"
    const extMatch = pattern.match(/\*(\.\w+)$/);
    const ext = extMatch ? extMatch[1] : null;
    // Extract prefix directory from pattern like "src/**/*.ts" → "src"
    const prefixMatch = pattern.match(/^([^*]+)\//);
    const prefix = prefixMatch ? prefixMatch[1] : '';

    const scanDir = prefix ? join(this.root, prefix) : this.root;
    const results: string[] = [];

    const walk = async (dir: string): Promise<void> => {
      let entries;
      try {
        entries = await readdir(dir, { withFileTypes: true });
      } catch { return; }
      for (const entry of entries) {
        const fullPath = join(dir, entry.name);
        if (entry.isDirectory()) {
          if (entry.name === 'node_modules' || entry.name === '.git') continue;
          await walk(fullPath);
        } else if (!ext || entry.name.endsWith(ext)) {
          const rel = fullPath.slice(this.root.length + 1);
          results.push(rel);
        }
      }
    };

    await walk(scanDir);
    return results.sort();
  }

  /** Resolve path and verify it stays within root — prevents directory traversal and symlink escapes */
  private safePath(filePath: string): string {
    const abs = resolve(join(this.root, filePath));
    // Quick check before expensive realpath (catches ../.. without I/O)
    if (!abs.startsWith(this.root)) {
      throw new PathTraversalError(filePath, this.root);
    }
    // Resolve symlinks to detect symlink-based escapes
    let real: string;
    try {
      real = realpathSync.native(abs);
    } catch {
      // File (or parent dirs) may not exist yet — walk up to the nearest existing ancestor
      let ancestor = dirname(abs);
      while (ancestor !== this.root && ancestor.startsWith(this.root)) {
        try {
          const ancestorReal = realpathSync.native(ancestor);
          if (!ancestorReal.startsWith(this.root)) {
            throw new PathTraversalError(filePath, this.root);
          }
          return abs;
        } catch {
          ancestor = dirname(ancestor);
        }
      }
      // Reached root or above — verify root itself resolves cleanly
      return abs;
    }
    if (!real.startsWith(this.root)) {
      throw new PathTraversalError(filePath, this.root);
    }
    return real;
  }
}
