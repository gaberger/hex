import { describe, it, expect } from 'vitest';
import { generateShortCode } from './shortened-url.js';

describe('generateShortCode', () => {
  it('is deterministic for a given seed', () => {
    expect(generateShortCode(12345)).toBe(generateShortCode(12345));
  });
  it('uses only url-safe base62 characters', () => {
    expect(generateShortCode(987654321)).toMatch(/^[0-9a-zA-Z]+$/);
  });
  it('encodes 0 as the first base62 digit "0"', () => {
    expect(generateShortCode(0)).toBe('0');
  });
  it('maps distinct seeds to distinct codes', () => {
    const codes = new Set([0, 1, 61, 62, 63, 100000].map(generateShortCode));
    expect(codes.size).toBe(6);
  });
});
