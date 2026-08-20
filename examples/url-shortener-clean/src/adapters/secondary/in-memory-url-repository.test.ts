import { describe, it, expect } from 'vitest';
import { InMemoryUrlRepository } from './in-memory-url-repository.js';

describe('InMemoryUrlRepository', () => {
  it('saves and finds by code', () => {
    const repo = new InMemoryUrlRepository();
    repo.save({ code: 'abc', longUrl: 'https://example.com' });
    expect(repo.findByCode('abc')).toEqual({ code: 'abc', longUrl: 'https://example.com' });
  });
  it('returns undefined for a missing code', () => {
    expect(new InMemoryUrlRepository().findByCode('nope')).toBeUndefined();
  });
  it('counts saved entries', () => {
    const repo = new InMemoryUrlRepository();
    repo.save({ code: 'a', longUrl: 'x' });
    repo.save({ code: 'b', longUrl: 'y' });
    expect(repo.count()).toBe(2);
  });
});
