import { Result } from '../../../shared/result.js';

/**
 * Represents a valid short URL identifier
 */
export class ShortId {
  private constructor(private readonly value: string) {}

  static create(value: string): Result<ShortId, string> {
    if (!value) {
      return Result.error('Short ID cannot be empty');
    }
    
    if (!/^[a-zA-Z0-9_-]+$/.test(value)) {
      return Result.error('Short ID can only contain alphanumeric characters, hyphens, and underscores');
    }
    
    if (value.length < 3 || value.length > 20) {
      return Result.error('Short ID must be between 3 and 20 characters');
    }
    
    return Result.success(new ShortId(value));
  }

  toString(): string {
    return this.value;
  }

  equals(other: ShortId): boolean {
    return this.value === other.value;
  }
}

/**
 * Represents a valid URL
 */
export class Url {
  private constructor(private readonly value: string) {}

  static create(value: string): Result<Url, string> {
    if (!value) {
      return Result.error('URL cannot be empty');
    }

    try {
      const url = new URL(value);
      if (!['http:', 'https:'].includes(url.protocol)) {
        return Result.error('URL must use HTTP or HTTPS protocol');
      }
      return Result.success(new Url(value));
    } catch {
      return Result.error('Invalid URL format');
    }
  }

  toString(): string {
    return this.value;
  }

  equals(other: Url): boolean {
    return this.value === other.value;
  }
}

/**
 * Core URL mapping entity representing the relationship between short ID and original URL
 */
export class UrlMapping {
  private constructor(
    private readonly shortId: ShortId,
    private readonly originalUrl: Url,
    private readonly createdAt: Date,
    private readonly clickCount: number = 0
  ) {}

  static create(shortId: ShortId, originalUrl: Url): UrlMapping {
    return new UrlMapping(shortId, originalUrl, new Date(), 0);
  }

  static restore(
    shortId: ShortId,
    originalUrl: Url,
    createdAt: Date,
    clickCount: number
  ): UrlMapping {
    return new UrlMapping(shortId, originalUrl, createdAt, clickCount);
  }

  getShortId(): ShortId {
    return this.shortId;
  }

  getOriginalUrl(): Url {
    return this.originalUrl;
  }

  getCreatedAt(): Date {
    return this.createdAt;
  }

  getClickCount(): number {
    return this.clickCount;
  }

  recordClick(): UrlMapping {
    return new UrlMapping(
      this.shortId,
      this.originalUrl,
      this.createdAt,
      this.clickCount + 1
    );
  }
}

/**
 * Domain service for URL shortening business logic
 */
export class UrlShorteningService {
  /**
   * Generates a random short ID
   */
  generateShortId(): ShortId {
    const characters = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789_-';
    let result = '';
    
    for (let i = 0; i < 8; i++) {
      result += characters.charAt(Math.floor(Math.random() * characters.length));
    }
    
    // This should always succeed given our generation logic
    const shortIdResult = ShortId.create(result);
    if (shortIdResult.isError()) {
      throw new Error('Failed to generate valid short ID');
    }
    
    return shortIdResult.getValue();
  }

  /**
   * Creates a URL mapping with validation
   */
  createMapping(shortId: ShortId, originalUrl: Url): UrlMapping {
    return UrlMapping.create(shortId, originalUrl);
  }

  /**
   * Validates if a URL can be shortened
   */
  canShortenUrl(url: Url): Result<void, string> {
    const urlString = url.toString();
    
    // Check for obvious localhost/private URLs that shouldn't be shortened
    if (urlString.includes('localhost') || urlString.includes('127.0.0.1')) {
      return Result.error('Cannot shorten localhost URLs');
    }
    
    return Result.success(undefined);
  }
}