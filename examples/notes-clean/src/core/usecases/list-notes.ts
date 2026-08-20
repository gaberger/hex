/**
 * Use case — list all persisted notes.
 *
 * core/usecases imports core/domain + core/ports only.
 */

import type { Note } from '../domain/note.js';
import type { INoteRepository } from '../ports/note-repository.js';

export class ListNotes {
  constructor(private readonly repo: INoteRepository) {}

  execute(): Note[] {
    return this.repo.list();
  }
}
