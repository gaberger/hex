/**
 * Filesystem secondary adapter -- implements IFileSystemPort.
 *
 * Uses Node/Bun fs APIs for file operations with path resolution
 * relative to a configurable root directory.
 */
import { access, mkdir, readFile, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import type { IFileSystemPort } from '../../core/ports/index.js';

export class FileSystemAdapter implements IFileSystemPort {
  private readonly root: string;

  constructor(rootPath?: string) {
    this.root = rootPath ? resolve(rootPath) : process.cwd();
  }

  async read(filePath: string): Promise<string> {
    const abs = this.resolve(filePath);
    return readFile(abs, { encoding: 'utf-8' });
  }

  async write(filePath: string, content: string): Promise<void> {
    const abs = this.resolve(filePath);
    await mkdir(dirname(abs), { recursive: true });
    await writeFile(abs, content, { encoding: 'utf-8' });
  }

  async exists(filePath: string): Promise<boolean> {
    try {
      await access(this.resolve(filePath));
      return true;
    } catch {
      return false;
    }
  }

  async glob(pattern: string): Promise<string[]> {
    const g = new Bun.Glob(pattern);
    const matches: string[] = [];
    for await (const entry of g.scan({ cwd: this.root, absolute: false })) {
      matches.push(entry);
    }
    return matches.sort();
  }

  private resolve(filePath: string): string {
    return join(this.root, filePath);
  }
}
