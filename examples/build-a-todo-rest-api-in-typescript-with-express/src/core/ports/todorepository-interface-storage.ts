export interface TodoRepository {
  create(todo: TodoCreateInput): Promise<Todo>;
  update(id: TodoId, updates: TodoUpdateInput): Promise<Todo>;
  delete(id: TodoId): Promise<void>;
  list(): Promise<Todo[]>;
}

export interface TodoCreateInput {
  name: string;
  description?: string;
}

export interface TodoUpdateInput {
  name?: string;
  description?: string;
}

export interface TodoId {
  id: string;
}

export interface Todo {
  id: string;
  name: string;
  description?: string;
  createdAt: string;
  updatedAt: string;
}

export interface TodoRepositoryError {
  code: string;
  message: string;
}

export class TodoNotFoundError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "TodoNotFoundError";
  }
}

export class InvalidTodoInputError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "InvalidTodoInputError";
  }
}