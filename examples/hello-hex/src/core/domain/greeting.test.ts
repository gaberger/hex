import { describe, it, expect } from 'bun:test';
import { createGreeting } from './greeting.js';

describe('createGreeting', () => {
  it('creates a greeting with the recipient name', () => {
    const greeting = createGreeting('Alice');
    expect(greeting.recipient).toBe('Alice');
    expect(greeting.message).toContain('Alice');
    expect(greeting.timestamp).toBeInstanceOf(Date);
  });

  it('produces a welcoming message', () => {
    const greeting = createGreeting('Bob');
    expect(greeting.message).toBe('Hello, Bob! Welcome to hex.');
  });
});
