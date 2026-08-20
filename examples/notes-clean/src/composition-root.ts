/**
 * Composition Root — the ONLY file that crosses adapter boundaries.
 *
 * This file wires concrete adapters to port interfaces.
 * No other file may import from adapters/ directly.
 */

import { CreateNote } from './core/usecases/create-note.js';
import { ListNotes } from './core/usecases/list-notes.js';
import { InMemoryNoteRepository } from './adapters/secondary/in-memory-note-repository.js';
import { UuidIdGenerator } from './adapters/secondary/uuid-id-generator.js';
import { SystemClock } from './adapters/secondary/system-clock.js';

export interface NotesApp {
  readonly createNote: CreateNote;
  readonly listNotes: ListNotes;
}

export function buildApp(): NotesApp {
  const repo = new InMemoryNoteRepository();
  const ids = new UuidIdGenerator();
  const clock = new SystemClock();

  return {
    createNote: new CreateNote(repo, ids, clock),
    listNotes: new ListNotes(repo),
  };
}
