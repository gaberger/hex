import { readFile, writeFile, rename, access } from 'node:fs/promises';
import { join, dirname } from 'node:path';
import type { TodoData } from '../../core/domain/entities.js';
import type { ITodoStoragePort } from '../../core/ports/index.js';

export class JsonStorageAdapter implements ITodoStoragePort {
  private readonly filePath: string;

  constructor(directory: string, filename = 'todos.json') {
    const sanitized = filename.replace(/[^a-zA-Z0-9._-]/g, '');
    this.filePath = join(directory, sanitized);
  }

  async load(): Promise<TodoData[]> {
    try {
      await access(this.filePath);
    } catch {
      return [];
    }
    const raw = await readFile(this.filePath, 'utf-8');
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      throw new Error('Invalid todos.json: expected an array');
    }
    return parsed as TodoData[];
  }

  async save(todos: TodoData[]): Promise<void> {
    const json = JSON.stringify(todos, null, 2);
    const tmpPath = this.filePath + '.tmp';
    await writeFile(tmpPath, json, 'utf-8');
    await rename(tmpPath, this.filePath);
  }
}
