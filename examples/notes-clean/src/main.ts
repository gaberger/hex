/**
 * Entry point — demonstrates create + list through the composed app.
 *
 * main.ts depends on the composition root only; it never touches adapters
 * or domain internals directly.
 */

import { buildApp } from './composition-root.js';

function main(): void {
  const app = buildApp();

  app.createNote.execute('Buy milk');
  app.createNote.execute('Ship the notes service');
  app.createNote.execute('Write more tests');

  const notes = app.listNotes.execute();

  console.log(`Notes (${notes.length}):`);
  for (const note of notes) {
    const when = new Date(note.createdAt).toISOString();
    console.log(`  [${note.id}] ${note.text} (created ${when})`);
  }
}

main();
