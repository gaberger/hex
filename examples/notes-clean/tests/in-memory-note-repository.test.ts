import { describe, it, expect } from 'vitest';
import { InMemoryNoteRepository } from '../src/adapters/secondary/in-memory-note-repository.js';
import { createNote } from '../src/core/domain/note.js';
import { ListNotes } from '../src/core/usecases/list-notes.js';

describe('InMemoryNoteRepository (adapter)', () => {
  it('persists saved notes and lists them in insertion order', () => {
    const repo = new InMemoryNoteRepository();
    repo.save(createNote('1', 'first', 10));
    repo.save(createNote('2', 'second', 20));

    expect(repo.list().map((n) => n.id)).toEqual(['1', '2']);
  });

  it('returns a defensive copy that cannot mutate internal state', () => {
    const repo = new InMemoryNoteRepository();
    repo.save(createNote('1', 'first', 10));

    repo.list().push(createNote('x', 'sneaky', 0));

    expect(repo.list()).toHaveLength(1);
  });

  it('works through the ListNotes use case', () => {
    const repo = new InMemoryNoteRepository();
    repo.save(createNote('1', 'only', 1));

    expect(new ListNotes(repo).execute()).toHaveLength(1);
  });
});
