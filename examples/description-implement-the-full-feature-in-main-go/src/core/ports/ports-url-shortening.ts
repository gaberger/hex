import { ShortCode } from '../domain/value-objects/short-code.js';
import { Url } from '../domain/value-objects/url.js';
import { ShortenedUrl } from '../domain/entities/shortened-url.js';

export interface UrlShorteningPort {
  shortenUrl(originalUrl: Url): Promise<ShortenedUrl>;
  retrieveUrl(shortCode: ShortCode): Promise<ShortenedUrl | null>;
}

export interface UrlStoragePort {
  save(shortenedUrl: ShortenedUrl): Promise<void>;
  findByShortCode(shortCode: ShortCode): Promise<ShortenedUrl | null>;
  findByOriginalUrl(originalUrl: Url): Promise<ShortenedUrl | null>;
}

export interface ShortCodeGeneratorPort {
  generate(): ShortCode;
}