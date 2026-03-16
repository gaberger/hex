import type { TodoData } from '../domain/entities.js';
import type { TodoId, TodoStatus, Priority } from '../domain/value-objects.js';

/** Output port (secondary/driven) — persistence */
export interface ITodoStoragePort {
  load(): Promise<TodoData[]>;
  save(todos: TodoData[]): Promise<void>;
}

/** Input port (primary/driving) — query side */
export interface ITodoQueryPort {
  getAll(): Promise<TodoData[]>;
  getById(id: TodoId): Promise<TodoData | null>;
  filter(status?: TodoStatus, priority?: Priority): Promise<TodoData[]>;
  stats(): Promise<{ total: number; pending: number; completed: number; rate: number }>;
}

/** Input port (primary/driving) — command side */
export interface ITodoCommandPort {
  create(title: string, priority?: Priority, tags?: string[]): Promise<TodoData>;
  complete(id: TodoId): Promise<TodoData>;
  update(
    id: TodoId,
    changes: Partial<Pick<TodoData, 'title' | 'priority' | 'tags'>>,
  ): Promise<TodoData>;
  delete(id: TodoId): Promise<void>;
}
