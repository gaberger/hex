export interface ShortUrl {
  readonly shortCode: string;
  readonly originalUrl: string;
  readonly createdAt: Date;
}

export class UrlShortener {
  private readonly urlStore = new Map<string, ShortUrl>();

  shortenUrl(originalUrl: string): ShortUrl {
    if (!this.isValidUrl(originalUrl)) {
      throw new Error('Invalid URL provided');
    }

    const shortCode = this.generateShortCode();
    const shortUrl: ShortUrl = {
      shortCode,
      originalUrl,
      createdAt: new Date()
    };

    this.urlStore.set(shortCode, shortUrl);
    return shortUrl;
  }

  getOriginalUrl(shortCode: string): string | null {
    const shortUrl = this.urlStore.get(shortCode);
    return shortUrl ? shortUrl.originalUrl : null;
  }

  private isValidUrl(url: string): boolean {
    try {
      new URL(url);
      return true;
    } catch {
      return false;
    }
  }

  private generateShortCode(): string {
    const chars = 'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789';
    let result = '';
    for (let i = 0; i < 6; i++) {
      result += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    return result;
  }
}