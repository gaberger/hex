export interface UrlShortenerRepository {
  save(originalUrl: string, shortCode: string): Promise<void>;
  findByShortCode(shortCode: string): Promise<string | null>;
  findByOriginalUrl(originalUrl: string): Promise<string | null>;
  exists(shortCode: string): Promise<boolean>;
}

export interface ShortCodeGenerator {
  generate(): string;
  generateWithLength(length: number): string;
}

export interface UrlValidator {
  isValid(url: string): boolean;
}

export interface UrlShortenerService {
  shortenUrl(originalUrl: string): Promise<{ shortCode: string; shortUrl: string }>;
  expandUrl(shortCode: string): Promise<string | null>;
  getStats(shortCode: string): Promise<{ originalUrl: string; createdAt: Date } | null>;
}