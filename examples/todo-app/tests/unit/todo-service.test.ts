import { describe, it, expect, beforeEach } from 'bun:test';
import { TodoService } from '../../src/core/usecases/todo-service.js';
import type { ITodoStoragePort } from '../../src/core/ports/index.js';
import type { TodoData } from '../../src/core/domain/entities.js';

class MockStorage implements ITodoStoragePort {
  private data: TodoData[] = [];
  loadCount = 0;
  saveCount = 0;

  constructor(initial: TodoData[] = []) {
    this.data = structuredClone(initial);
  }

  async load(): Promise<TodoData[]> {
    this.loadCount++;
    return structuredClone(this.data);
  }

  async save(todos: TodoData[]): Promise<void> {
    this.saveCount++;
    this.data = structuredClone(todos);
  }

  getSaved(): TodoData[] {
    return this.data;
  }
}

describe('TodoService', () => {
  let storage: MockStorage;
  let service: TodoService;

  beforeEach(() => {
    storage = new MockStorage();
    service = new TodoService(storage);
  });

  describe('create', () => {
    it('creates a todo and persists it', async () => {
      const result = await service.create('Buy groceries', 'high', ['shopping']);

      expect(result.title).toBe('Buy groceries');
      expect(result.priority).toBe('high');
      expect(result.status).toBe('pending');
      expect(result.tags).toEqual(['shopping']);
      expect(storage.saveCount).toBe(1);
    });

    it('uses default priority and tags', async () => {
      const result = await service.create('Simple task');
      expect(result.priority).toBe('medium');
      expect(result.tags).toEqual([]);
    });

    it('throws on empty title', async () => {
      await expect(service.create('')).rejects.toThrow('Todo title cannot be empty');
    });
  });

  describe('complete', () => {
    it('marks a todo as completed', async () => {
      const created = await service.create('Task');
      const completed = await service.complete(created.id);

      expect(completed.status).toBe('completed');
      expect(completed.completedAt).toBeGreaterThan(0);
    });

    it('throws on non-existent ID', async () => {
      await expect(service.complete('nonexistent')).rejects.toThrow('Todo not found');
    });
  });

  describe('update', () => {
    it('updates title', async () => {
      const created = await service.create('Old');
      const updated = await service.update(created.id, { title: 'New' });
      expect(updated.title).toBe('New');
    });

    it('updates priority', async () => {
      const created = await service.create('Task');
      const updated = await service.update(created.id, { priority: 'high' });
      expect(updated.priority).toBe('high');
    });

    it('throws on non-existent ID', async () => {
      await expect(service.update('nonexistent', { title: 'X' })).rejects.toThrow('Todo not found');
    });
  });

  describe('delete', () => {
    it('removes a todo', async () => {
      const created = await service.create('Task');
      await service.delete(created.id);
      const all = await service.getAll();
      expect(all).toHaveLength(0);
    });

    it('throws on non-existent ID', async () => {
      await expect(service.delete('nonexistent')).rejects.toThrow('Todo not found');
    });
  });

  describe('getAll', () => {
    it('returns all todos', async () => {
      await service.create('One');
      await service.create('Two');
      const all = await service.getAll();
      expect(all).toHaveLength(2);
    });

    it('loads from storage only once', async () => {
      await service.getAll();
      await service.getAll();
      expect(storage.loadCount).toBe(1);
    });
  });

  describe('getById', () => {
    it('returns todo by ID', async () => {
      const created = await service.create('Task');
      const found = await service.getById(created.id);
      expect(found).not.toBeNull();
      expect(found!.title).toBe('Task');
    });

    it('returns null for unknown ID', async () => {
      const found = await service.getById('unknown');
      expect(found).toBeNull();
    });
  });

  describe('filter', () => {
    it('filters by status', async () => {
      const t1 = await service.create('Pending');
      const t2 = await service.create('Done');
      await service.complete(t2.id);

      const pending = await service.filter('pending');
      expect(pending).toHaveLength(1);
      expect(pending[0].title).toBe('Pending');
    });

    it('filters by priority', async () => {
      await service.create('Low', 'low');
      await service.create('High', 'high');

      const high = await service.filter(undefined, 'high');
      expect(high).toHaveLength(1);
      expect(high[0].title).toBe('High');
    });
  });

  describe('stats', () => {
    it('calculates stats correctly', async () => {
      await service.create('One');
      const t2 = await service.create('Two');
      await service.complete(t2.id);

      const stats = await service.stats();
      expect(stats.total).toBe(2);
      expect(stats.pending).toBe(1);
      expect(stats.completed).toBe(1);
      expect(stats.rate).toBe(0.5);
    });

    it('returns zeros for empty list', async () => {
      const stats = await service.stats();
      expect(stats.total).toBe(0);
      expect(stats.pending).toBe(0);
      expect(stats.rate).toBe(0);
    });
  });
});
