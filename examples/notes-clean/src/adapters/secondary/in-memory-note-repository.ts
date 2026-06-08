/**
 * Secondary adapter — in-memory implementation of INoteRepository.
 *
 * Adapters import ports only.
 */

import type { Note } from '../../core/domain/note.js';
import type { INoteRepository } from '../../core/ports/note-repository.js';

export class InMemoryNoteRepository implements INoteRepository {
  private readonly notes: Note[] = [];

  save(note: Note): void {
    this.notes.push(note);
  }

  list(): Note[] {
    // Defensive copy so callers cannot mutate internal state.
    return [...this.notes];
  }
}
