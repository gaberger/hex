import { describe, it, expect } from 'vitest';
import { createNote } from '../src/core/domain/note.js';

describe('createNote (domain)', () => {
  it('builds a note with id, trimmed text, and timestamp', () => {
    const note = createNote('id-1', '  hello  ', 1234);
    expect(note).toEqual({ id: 'id-1', text: 'hello', createdAt: 1234 });
  });

  it('rejects empty/blank text', () => {
    expect(() => createNote('id-1', '   ', 1)).toThrow(/text must not be empty/);
  });

  it('rejects empty id', () => {
    expect(() => createNote('  ', 'hello', 1)).toThrow(/id must not be empty/);
  });
});
