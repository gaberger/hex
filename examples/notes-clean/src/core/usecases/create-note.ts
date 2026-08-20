/**
 * Use case — create a note and persist it.
 *
 * core/usecases imports core/domain + core/ports only.
 */

import { createNote, type Note } from '../domain/note.js';
import type { INoteRepository } from '../ports/note-repository.js';
import type { IIdGenerator } from '../ports/id-generator.js';
import type { IClock } from '../ports/clock.js';

export class CreateNote {
  constructor(
    private readonly repo: INoteRepository,
    private readonly ids: IIdGenerator,
    private readonly clock: IClock,
  ) {}

  execute(text: string): Note {
    const note = createNote(this.ids.next(), text, this.clock.now());
    this.repo.save(note);
    return note;
  }
}
