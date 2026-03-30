import type { ShortUrl } from '../domain/entities/short-url.js';
import type { OriginalUrl } from '../domain/value-objects/original-url.js';
import type { ShortCode } from '../domain/value-objects/short-code.js';

export interface UrlShortenerPort {
  createShortUrl(originalUrl: OriginalUrl): Promise<ShortUrl>;
  retrieveOriginalUrl(shortCode: ShortCode): Promise<OriginalUrl | null>;
  urlExists(shortCode: ShortCode): Promise<boolean>;
}

export interface UrlRepositoryPort {
  save(shortUrl: ShortUrl): Promise<void>;
  findByShortCode(shortCode: ShortCode): Promise<ShortUrl | null>;
  findByOriginalUrl(originalUrl: OriginalUrl): Promise<ShortUrl | null>;
}

export interface ShortCodeGeneratorPort {
  generate(): Promise<ShortCode>;
  isUnique(shortCode: ShortCode): Promise<boolean>;
}