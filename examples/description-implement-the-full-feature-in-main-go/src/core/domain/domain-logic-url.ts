export class ShortCode {
  private readonly value: string;

  private constructor(value: string) {
    this.value = value;
  }

  static create(value: string): ShortCode {
    if (!value || value.length === 0) {
      throw new Error('Short code cannot be empty');
    }
    
    if (value.length > 10) {
      throw new Error('Short code cannot exceed 10 characters');
    }
    
    if (!/^[a-zA-Z0-9_-]+$/.test(value)) {
      throw new Error('Short code can only contain alphanumeric characters, hyphens, and underscores');
    }
    
    return new ShortCode(value);
  }

  static generate(): ShortCode {
    const chars = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
    let result = '';
    for (let i = 0; i < 6; i++) {
      result += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    return new ShortCode(result);
  }

  toString(): string {
    return this.value;
  }

  equals(other: ShortCode): boolean {
    return this.value === other.value;
  }
}

export class OriginalUrl {
  private readonly value: string;

  private constructor(value: string) {
    this.value = value;
  }

  static create(value: string): OriginalUrl {
    if (!value || value.length === 0) {
      throw new Error('URL cannot be empty');
    }

    try {
      new URL(value);
    } catch {
      throw new Error('Invalid URL format');
    }

    return new OriginalUrl(value);
  }

  toString(): string {
    return this.value;
  }

  equals(other: OriginalUrl): boolean {
    return this.value === other.value;
  }
}

export class UrlMapping {
  private readonly shortCode: ShortCode;
  private readonly originalUrl: OriginalUrl;
  private readonly createdAt: Date;
  private clickCount: number;

  constructor(shortCode: ShortCode, originalUrl: OriginalUrl, createdAt?: Date) {
    this.shortCode = shortCode;
    this.originalUrl = originalUrl;
    this.createdAt = createdAt || new Date();
    this.clickCount = 0;
  }

  static create(originalUrl: OriginalUrl, shortCode?: ShortCode): UrlMapping {
    const code = shortCode || ShortCode.generate();
    return new UrlMapping(code, originalUrl);
  }

  getShortCode(): ShortCode {
    return this.shortCode;
  }

  getOriginalUrl(): OriginalUrl {
    return this.originalUrl;
  }

  getCreatedAt(): Date {
    return this.createdAt;
  }

  getClickCount(): number {
    return this.clickCount;
  }

  incrementClickCount(): void {
    this.clickCount++;
  }

  equals(other: UrlMapping): boolean {
    return this.shortCode.equals(other.shortCode);
  }
}