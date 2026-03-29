import{ validateTodo } from '../../../src/core/domain/todo-entity-validation.js';

describe('validateTodo', () => {
  describe('happy path', () => {
    it('returns true for valid todo with description under 255 chars', () => {
      const todo = { title: 'Valid Title', description: 'Short description' };
      expect(validateTodo(todo)).toBe(true);
    });

    it('returns true for valid todo with empty description', () => {
      const todo = { title: 'Valid Title', description: undefined };
      expect(validateTodo(todo)).toBe(true);
    });

    it('returns true for valid todo with description exactly 255 chars', () => {
      const description = 'a'.repeat(255);
      const todo = { title: 'Valid Title', description };
      expect(validateTodo(todo)).toBe(true);
    });
  });

  describe('error cases', () => {
    it('returns false for empty title', () => {
      const todo = { title: '', description: 'Valid' };
      expect(validateTodo(todo)).toBe(false);
    });

    it('returns false for title with only whitespace', () => {
      const todo = { title: '   ', description: 'Valid' };
      expect(validateTodo(todo)).toBe(false);
    });

    it('returns false for description over 255 chars', () => {
      const description = 'a'.repeat(256);
      const todo = { title: 'Valid Title', description };
      expect(validateTodo(todo)).toBe(false);
    });
  });

  describe('edge cases', () => {
    it('returns false for null title', () => {
      const todo = { title: null, description: 'Valid' };
      expect(validateTodo(todo)).toBe(false);
    });

    it('returns false for undefined title', () => {
      const todo = { title: undefined, description: 'Valid' };
      expect(validateTodo(todo)).toBe(false);
    });

    it('returns false for title with Unicode characters', () => {
      const todo = { title: 'Valid Title with 한글', description: 'Valid' };
      expect(validateTodo(todo)).toBe(true);
    });
  });
});