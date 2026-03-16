import { Todo, TodoList } from '../domain/entities.js';
import type { TodoData } from '../domain/entities.js';
import type { TodoId, TodoStatus, Priority } from '../domain/value-objects.js';
import { NotFoundError } from '../domain/errors.js';
import type {
  ITodoStoragePort,
  ITodoQueryPort,
  ITodoCommandPort,
} from '../ports/index.js';
import type { ILoggerPort } from '../ports/logger.js';

export class TodoService implements ITodoQueryPort, ITodoCommandPort {
  private list: TodoList | null = null;

  constructor(
    private readonly storage: ITodoStoragePort,
    private readonly logger?: ILoggerPort,
  ) {}

  private async ensureLoaded(): Promise<TodoList> {
    if (!this.list) {
      const data = await this.storage.load();
      this.list = TodoList.fromData(data);
    }
    return this.list;
  }

  private async persist(): Promise<void> {
    if (!this.list) return;
    await this.storage.save(this.list.toDataList());
  }

  // -- Query --

  async getAll(): Promise<TodoData[]> {
    const list = await this.ensureLoaded();
    return list.toDataList();
  }

  async getById(id: TodoId): Promise<TodoData | null> {
    const list = await this.ensureLoaded();
    const todo = list.getById(id);
    return todo ? todo.toData() : null;
  }

  async filter(status?: TodoStatus, priority?: Priority): Promise<TodoData[]> {
    const list = await this.ensureLoaded();
    return list.filter(status, priority).map((t) => t.toData());
  }

  async stats(): Promise<{ total: number; pending: number; completed: number; rate: number }> {
    const list = await this.ensureLoaded();
    return {
      total: list.total,
      pending: list.pendingCount,
      completed: list.completedCount,
      rate: Math.round(list.completionRate * 100) / 100,
    };
  }

  // -- Command --

  async create(
    title: string,
    priority: Priority = 'medium',
    tags: string[] = [],
  ): Promise<TodoData> {
    const list = await this.ensureLoaded();
    const todo = Todo.create(title, priority, tags);
    list.add(todo);
    await this.persist();
    this.logger?.info('Todo created', { id: todo.id, title });
    return todo.toData();
  }

  async complete(id: TodoId): Promise<TodoData> {
    const list = await this.ensureLoaded();
    const existing = list.getById(id);
    if (!existing) throw new NotFoundError('Todo', id);
    const completed = existing.complete();
    list.replace(completed);
    await this.persist();
    this.logger?.info('Todo completed', { id });
    return completed.toData();
  }

  async update(
    id: TodoId,
    changes: Partial<Pick<TodoData, 'title' | 'priority' | 'tags'>>,
  ): Promise<TodoData> {
    const list = await this.ensureLoaded();
    let todo = list.getById(id);
    if (!todo) throw new NotFoundError('Todo', id);
    if (changes.title !== undefined) {
      todo = todo.updateTitle(changes.title);
    }
    if (changes.priority !== undefined) {
      todo = todo.setPriority(changes.priority);
    }
    if (changes.tags !== undefined) {
      const current = new Set(todo.tags);
      for (const tag of changes.tags) {
        if (!current.has(tag)) {
          todo = todo.addTag(tag);
        }
      }
    }
    list.replace(todo);
    await this.persist();
    return todo.toData();
  }

  async delete(id: TodoId): Promise<void> {
    const list = await this.ensureLoaded();
    const existed = list.remove(id);
    if (!existed) throw new NotFoundError('Todo', id);
    await this.persist();
  }
}
