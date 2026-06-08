import { describe, it, expect } from 'vitest';
import { ShortenUrl } from './shorten-url.js';
import { IUrlRepository } from '../ports/url-repository.js';
import { ShortenedUrl } from '../domain/shortened-url.js';

// Test double implementing the port — keeps the usecases layer free of adapter imports.
class FakeRepo implements IUrlRepository {
  private m = new Map<string, ShortenedUrl>();
  save(e: ShortenedUrl) { this.m.set(e.code, e); }
  findByCode(c: string) { return this.m.get(c); }
  count() { return this.m.size; }
}

describe('ShortenUrl', () => {
  it('shortens a url and resolves it back to the original', () => {
    const uc = new ShortenUrl(new FakeRepo());
    const code = uc.shorten('https://example.com');
    expect(typeof code).toBe('string');
    expect(uc.resolve(code)).toBe('https://example.com');
  });
  it('generates distinct codes for distinct urls', () => {
    const uc = new ShortenUrl(new FakeRepo());
    expect(uc.shorten('https://a.com')).not.toBe(uc.shorten('https://b.com'));
  });
  it('returns undefined when resolving an unknown code', () => {
    expect(new ShortenUrl(new FakeRepo()).resolve('zzz')).toBeUndefined();
  });
});
