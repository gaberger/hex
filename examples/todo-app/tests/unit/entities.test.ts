import { describe, it, expect } from 'bun:test';
import { Todo, TodoList } from '../../src/core/domain/entities.js';
import type { TodoData } from '../../src/core/domain/entities.js';

describe('Todo', () => {
  it('creates a todo with valid title', () => {
    const todo = Todo.create('Buy groceries', 'high', ['shopping']);
    expect(todo.title).toBe('Buy groceries');
    expect(todo.status).toBe('pending');
    expect(todo.priority).toBe('high');
    expect(todo.tags).toEqual(['shopping']);
    expect(todo.id).toBeDefined();
    expect(todo.createdAt).toBeGreaterThan(0);
  });

  it('trims whitespace from title', () => {
    const todo = Todo.create('  padded title  ');
    expect(todo.title).toBe('padded title');
  });

  it('throws on empty title', () => {
    expect(() => Todo.create('')).toThrow('Todo title cannot be empty');
  });

  it('throws on whitespace-only title', () => {
    expect(() => Todo.create('   ')).toThrow('Todo title cannot be empty');
  });

  it('throws on title exceeding 200 chars', () => {
    const longTitle = 'a'.repeat(201);
    expect(() => Todo.create(longTitle)).toThrow('cannot exceed 200 characters');
  });

  it('complete() returns new immutable Todo with completed status', () => {
    const original = Todo.create('Task');
    const completed = original.complete();

    expect(completed.status).toBe('completed');
    expect(completed.completedAt).toBeGreaterThan(0);
    // Original unchanged
    expect(original.status).toBe('pending');
    expect(original.completedAt).toBeUndefined();
    // Same identity
    expect(completed.id).toBe(original.id);
  });

  it('throws when completing already completed todo', () => {
    const todo = Todo.create('Task');
    const completed = todo.complete();
    expect(() => completed.complete()).toThrow('already completed');
  });

  it('updateTitle() returns new Todo with updated title', () => {
    const original = Todo.create('Old title');
    const updated = original.updateTitle('New title');
    expect(updated.title).toBe('New title');
    expect(original.title).toBe('Old title');
  });

  it('setPriority() returns new Todo with new priority', () => {
    const todo = Todo.create('Task', 'low');
    const updated = todo.setPriority('high');
    expect(updated.priority).toBe('high');
    expect(todo.priority).toBe('low');
  });

  it('addTag() returns new Todo with added tag', () => {
    const todo = Todo.create('Task');
    const tagged = todo.addTag('urgent');
    expect(tagged.tags).toEqual(['urgent']);
    expect(todo.tags).toEqual([]);
  });

  it('addTag() skips duplicate tags', () => {
    const todo = Todo.create('Task', 'medium', ['urgent']);
    const same = todo.addTag('urgent');
    expect(same).toBe(todo); // Returns same instance
  });

  it('addTag() throws on empty tag', () => {
    const todo = Todo.create('Task');
    expect(() => todo.addTag('')).toThrow('Tag cannot be empty');
  });

  it('drainEvents() returns accumulated events and clears them', () => {
    const todo = Todo.create('Task', 'high');
    const events = todo.drainEvents();
    expect(events).toHaveLength(1);
    expect(events[0].type).toBe('TodoCreated');
    expect(todo.drainEvents()).toHaveLength(0);
  });

  it('toData() returns a plain data object', () => {
    const todo = Todo.create('Task', 'medium', ['a']);
    const data = todo.toData();
    expect(data.id).toBe(todo.id);
    expect(data.title).toBe('Task');
    expect(data.status).toBe('pending');
    expect(data.tags).toEqual(['a']);
  });
});

describe('TodoList', () => {
  function makeTodo(title: string, status: TodoData['status'] = 'pending', priority: TodoData['priority'] = 'medium'): Todo {
    const todo = Todo.create(title, priority);
    if (status === 'completed') return todo.complete();
    return todo;
  }

  it('adds and retrieves todos', () => {
    const list = new TodoList();
    const todo = Todo.create('Task');
    list.add(todo);
    expect(list.getById(todo.id)).toBeDefined();
    expect(list.total).toBe(1);
  });

  it('removes todos', () => {
    const list = new TodoList();
    const todo = Todo.create('Task');
    list.add(todo);
    expect(list.remove(todo.id)).toBe(true);
    expect(list.total).toBe(0);
  });

  it('returns false when removing non-existent id', () => {
    const list = new TodoList();
    expect(list.remove('nonexistent')).toBe(false);
  });

  it('filters by status', () => {
    const list = new TodoList();
    list.add(Todo.create('Pending'));
    const completed = Todo.create('Done').complete();
    list.add(completed);

    const pending = list.filter('pending');
    expect(pending).toHaveLength(1);
    expect(pending[0].title).toBe('Pending');
  });

  it('filters by priority', () => {
    const list = new TodoList();
    list.add(Todo.create('Low', 'low'));
    list.add(Todo.create('High', 'high'));

    const high = list.filter(undefined, 'high');
    expect(high).toHaveLength(1);
    expect(high[0].title).toBe('High');
  });

  it('calculates stats correctly', () => {
    const list = new TodoList();
    list.add(Todo.create('One'));
    list.add(Todo.create('Two'));
    const done = Todo.create('Three').complete();
    list.add(done);

    expect(list.pendingCount).toBe(2);
    expect(list.completedCount).toBe(1);
    expect(list.total).toBe(3);
    expect(list.completionRate).toBeCloseTo(1 / 3);
  });

  it('completionRate is 0 for empty list', () => {
    const list = new TodoList();
    expect(list.completionRate).toBe(0);
  });

  it('fromData() reconstructs from plain data', () => {
    const original = Todo.create('Task', 'high', ['tag']);
    const data = [original.toData()];
    const list = TodoList.fromData(data);
    expect(list.total).toBe(1);
    expect(list.getAll()[0].title).toBe('Task');
  });
});
