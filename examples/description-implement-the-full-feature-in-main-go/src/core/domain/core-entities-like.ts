export class ShortCode {
  private constructor(private readonly value: string) {}

  static create(value: string): ShortCode {
    if (!value) {
      throw new Error('Short code cannot be empty');
    }
    if (!/^[a-zA-Z0-9_-]+$/.test(value)) {
      throw new Error('Short code contains invalid characters');
    }
    if (value.length > 50) {
      throw new Error('Short code too long');
    }
    return new ShortCode(value);
  }

  static generate(): ShortCode {
    const chars = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
    let result = '';
    for (let i = 0; i < 8; i++) {
      result += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    return new ShortCode(result);
  }

  getValue(): string {
    return this.value;
  }

  equals(other: ShortCode): boolean {
    return this.value === other.value;
  }
}

export class OriginalUrl {
  private constructor(private readonly value: string) {}

  static create(value: string): OriginalUrl {
    if (!value) {
      throw new Error('URL cannot be empty');
    }
    
    try {
      new URL(value);
    } catch {
      throw new Error('Invalid URL format');
    }

    if (value.length > 2048) {
      throw new Error('URL too long');
    }

    return new OriginalUrl(value);
  }

  getValue(): string {
    return this.value;
  }

  equals(other: OriginalUrl): boolean {
    return this.value === other.value;
  }
}

export class UrlMapping {
  constructor(
    private readonly shortCode: ShortCode,
    private readonly originalUrl: OriginalUrl,
    private readonly createdAt: Date = new Date()
  ) {}

  getShortCode(): ShortCode {
    return this.shortCode;
  }

  getOriginalUrl(): OriginalUrl {
    return this.originalUrl;
  }

  getCreatedAt(): Date {
    return this.createdAt;
  }

  static create(shortCode: ShortCode, originalUrl: OriginalUrl): UrlMapping {
    return new UrlMapping(shortCode, originalUrl);
  }
}

// Added export to comply with module resolution
export class UrlShortener {
  shortenUrl(originalUrl: OriginalUrl): UrlMapping {
    const shortCode = ShortCode.generate();
    return UrlMapping.create(shortCode, originalUrl);
  }

  expandUrl(shortCode: ShortCode, mapping: UrlMapping): OriginalUrl {
    if (!shortCode.equals(mapping.getShortCode())) {
      throw new Error('Short code does not match mapping');
    }
    return mapping.getOriginalUrl();
  }
}