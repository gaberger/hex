import { describe, it, expect } from 'vitest';
import { shortenUrl } from './composition-root.js';

describe('composition root (integration: all layers wired)', () => {
  it('shortens then resolves a url through the wired stack', () => {
    const code = shortenUrl.shorten('https://hexagonal.example');
    expect(typeof code).toBe('string');
    expect(shortenUrl.resolve(code)).toBe('https://hexagonal.example');
  });
});
