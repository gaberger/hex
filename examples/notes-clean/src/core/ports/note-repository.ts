/**
 * Port — persistence boundary for notes.
 *
 * core/ports imports core/domain only (for value types).
 */

import type { Note } from '../domain/note.js';

export interface INoteRepository {
  save(note: Note): void;
  list(): Note[];
}
