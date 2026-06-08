import { describe, it, expect } from 'vitest';
import { CreateNote } from '../src/core/usecases/create-note.js';
import type { INoteRepository } from '../src/core/ports/note-repository.js';
import type { Note } from '../src/core/domain/note.js';

function fakeRepo(): INoteRepository & { saved: Note[] } {
  const saved: Note[] = [];
  return {
    saved,
    save: (n) => { saved.push(n); },
    list: () => [...saved],
  };
}

describe('CreateNote (use case)', () => {
  it('generates id + timestamp from ports and persists the note', () => {
    const repo = fakeRepo();
    const useCase = new CreateNote(repo, { next: () => 'fixed-id' }, { now: () => 999 });

    const note = useCase.execute('hi there');

    expect(note).toEqual({ id: 'fixed-id', text: 'hi there', createdAt: 999 });
    expect(repo.saved).toHaveLength(1);
    expect(repo.saved[0]).toEqual(note);
  });

  it('uses a fresh id per call', () => {
    const repo = fakeRepo();
    let n = 0;
    const useCase = new CreateNote(repo, { next: () => `id-${++n}` }, { now: () => 0 });

    useCase.execute('a');
    useCase.execute('b');

    expect(repo.saved.map((x) => x.id)).toEqual(['id-1', 'id-2']);
  });
});
