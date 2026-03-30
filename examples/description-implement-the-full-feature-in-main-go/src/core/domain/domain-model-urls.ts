export interface Url {
  readonly id: string;
  readonly originalUrl: string;
  readonly shortCode: string;
  readonly createdAt: Date;
  readonly expiresAt?: Date;
  readonly clickCount: number;
}

export class UrlEntity implements Url {
  constructor(
    public readonly id: string,
    public readonly originalUrl: string,
    public readonly shortCode: string,
    public readonly createdAt: Date,
    public readonly expiresAt?: Date,
    public readonly clickCount: number = 0
  ) {
    this.validateOriginalUrl(originalUrl);
    this.validateShortCode(shortCode);
  }

  private validateOriginalUrl(url: string): void {
    if (!url || url.trim().length === 0) {
      throw new Error('Original URL cannot be empty');
    }
    
    try {
      new URL(url);
    } catch {
      throw new Error('Invalid URL format');
    }
  }

  private validateShortCode(code: string): void {
    if (!code || code.trim().length === 0) {
      throw new Error('Short code cannot be empty');
    }
    
    if (!/^[a-zA-Z0-9_-]+$/.test(code)) {
      throw new Error('Short code can only contain alphanumeric characters, hyphens, and underscores');
    }
  }

  public isExpired(): boolean {
    if (!this.expiresAt) {
      return false;
    }
    return new Date() > this.expiresAt;
  }

  public incrementClickCount(): UrlEntity {
    return new UrlEntity(
      this.id,
      this.originalUrl,
      this.shortCode,
      this.createdAt,
      this.expiresAt,
      this.clickCount + 1
    );
  }

  public static create(
    id: string,
    originalUrl: string,
    shortCode: string,
    expiresAt?: Date
  ): UrlEntity {
    return new UrlEntity(
      id,
      originalUrl,
      shortCode,
      new Date(),
      expiresAt
    );
  }
}

export class ShortCodeGenerator {
  private static readonly CHARACTERS = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
  private static readonly DEFAULT_LENGTH = 6;

  public static generate(length: number = this.DEFAULT_LENGTH): string {
    let result = '';
    for (let i = 0; i < length; i++) {
      result += this.CHARACTERS.charAt(Math.floor(Math.random() * this.CHARACTERS.length));
    }
    return result;
  }

  public static isValid(shortCode: string): boolean {
    if (!shortCode || shortCode.trim().length === 0) {
      return false;
    }
    return /^[a-zA-Z0-9_-]+$/.test(shortCode);
  }
}

export type UrlCreationParams = {
  originalUrl: string;
  customShortCode?: string;
  expiresAt?: Date;
};

export class UrlDomainService {
  public createUrl(params: UrlCreationParams): UrlEntity {
    const shortCode = params.customShortCode || ShortCodeGenerator.generate();
    const id = this.generateId();
    
    return UrlEntity.create(id, params.originalUrl, shortCode, params.expiresAt);
  }

  private generateId(): string {
    return `url_${Date.now()}_${Math.random().toString(36).substr(2, 9)}`;
  }
}