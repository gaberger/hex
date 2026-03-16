import type {
  TodoId,
  TodoStatus,
  Priority,
} from './value-objects.js';
import { createTodoId, createTodoTitle } from './value-objects.js';

export type TodoEvent =
  | { type: 'TodoCreated'; payload: { id: TodoId; title: string; priority: Priority } }
  | { type: 'TodoCompleted'; payload: { id: TodoId } }
  | { type: 'TodoDeleted'; payload: { id: TodoId } }
  | { type: 'TodoUpdated'; payload: { id: TodoId; changes: Partial<TodoData> } };

export interface TodoData {
  readonly id: TodoId;
  readonly title: string;
  readonly status: TodoStatus;
  readonly priority: Priority;
  readonly createdAt: number;
  readonly completedAt?: number;
  readonly tags: readonly string[];
}

export class Todo {
  private readonly events: TodoEvent[] = [];

  constructor(private readonly data: TodoData) {
    this.validate();
  }

  static create(
    title: string,
    priority: Priority = 'medium',
    tags: string[] = [],
  ): Todo {
    const todoTitle = createTodoTitle(title);
    const data: TodoData = {
      id: createTodoId(),
      title: todoTitle.value,
      status: 'pending',
      priority,
      createdAt: Date.now(),
      tags,
    };
    const todo = new Todo(data);
    todo.events.push({
      type: 'TodoCreated',
      payload: { id: data.id, title: data.title, priority },
    });
    return todo;
  }

  get id(): TodoId { return this.data.id; }
  get title(): string { return this.data.title; }
  get status(): TodoStatus { return this.data.status; }
  get priority(): Priority { return this.data.priority; }
  get createdAt(): number { return this.data.createdAt; }
  get completedAt(): number | undefined { return this.data.completedAt; }
  get tags(): readonly string[] { return this.data.tags; }

  toData(): TodoData {
    return { ...this.data, tags: [...this.data.tags] };
  }

  drainEvents(): TodoEvent[] {
    return this.events.splice(0, this.events.length);
  }

  complete(): Todo {
    if (this.data.status === 'completed') {
      throw new Error(`Todo ${this.data.id} is already completed`);
    }
    const next = new Todo({
      ...this.data,
      status: 'completed',
      completedAt: Date.now(),
    });
    next.events.push({ type: 'TodoCompleted', payload: { id: this.data.id } });
    return next;
  }

  updateTitle(newTitle: string): Todo {
    const title = createTodoTitle(newTitle);
    const next = new Todo({ ...this.data, title: title.value });
    next.events.push({
      type: 'TodoUpdated',
      payload: { id: this.data.id, changes: { title: title.value } },
    });
    return next;
  }

  setPriority(priority: Priority): Todo {
    const next = new Todo({ ...this.data, priority });
    next.events.push({
      type: 'TodoUpdated',
      payload: { id: this.data.id, changes: { priority } },
    });
    return next;
  }

  addTag(tag: string): Todo {
    const trimmed = tag.trim();
    if (trimmed.length === 0) throw new Error('Tag cannot be empty');
    if (this.data.tags.includes(trimmed)) return this;
    const next = new Todo({
      ...this.data,
      tags: [...this.data.tags, trimmed],
    });
    next.events.push({
      type: 'TodoUpdated',
      payload: { id: this.data.id, changes: { tags: next.data.tags } },
    });
    return next;
  }

  private validate(): void {
    createTodoTitle(this.data.title);
  }
}

export class TodoList {
  private items: Map<TodoId, Todo>;

  constructor(todos: Todo[] = []) {
    this.items = new Map(todos.map((t) => [t.id, t]));
  }

  static fromData(dataList: TodoData[]): TodoList {
    return new TodoList(dataList.map((d) => new Todo(d)));
  }

  add(todo: Todo): void {
    this.items.set(todo.id, todo);
  }

  remove(id: TodoId): boolean {
    return this.items.delete(id);
  }

  getById(id: TodoId): Todo | undefined {
    return this.items.get(id);
  }

  replace(todo: Todo): void {
    this.items.set(todo.id, todo);
  }

  getAll(): Todo[] {
    return Array.from(this.items.values());
  }

  filter(status?: TodoStatus, priority?: Priority): Todo[] {
    return this.getAll().filter((t) => {
      if (status && t.status !== status) return false;
      if (priority && t.priority !== priority) return false;
      return true;
    });
  }

  toDataList(): TodoData[] {
    return this.getAll().map((t) => t.toData());
  }

  get pendingCount(): number {
    return this.getAll().filter((t) => t.status === 'pending').length;
  }

  get completedCount(): number {
    return this.getAll().filter((t) => t.status === 'completed').length;
  }

  get total(): number {
    return this.items.size;
  }

  get completionRate(): number {
    if (this.total === 0) return 0;
    return this.completedCount / this.total;
  }
}
