/**
 * Domain — pure note value type and factory.
 *
 * core/domain imports only core/domain.
 */

export interface Note {
  readonly id: string;
  readonly text: string;
  readonly createdAt: number;
}

/**
 * Construct a valid Note. Rejects empty/blank text so an invalid
 * note can never exist in the domain.
 */
export function createNote(id: string, text: string, createdAt: number): Note {
  const trimmed = text.trim();
  if (trimmed.length === 0) {
    throw new Error('Note text must not be empty');
  }
  if (id.trim().length === 0) {
    throw new Error('Note id must not be empty');
  }
  return { id, text: trimmed, createdAt };
}
