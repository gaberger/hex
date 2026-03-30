import { ShortCode } from './value-objects/short-code.js';
import { OriginalUrl } from './value-objects/original-url.js';
import { ShortenedUrl } from './entities/shortened-url.js';

export class CoreLogicUrl {
  private urls: Map<string, ShortenedUrl> = new Map();
  private reverseIndex: Map<string, string> = new Map();

  shortenUrl(originalUrl: OriginalUrl): ShortenedUrl {
    // Check if URL already exists
    const existingShortCode = this.reverseIndex.get(originalUrl.value);
    if (existingShortCode) {
      const existing = this.urls.get(existingShortCode);
      if (existing) {
        return existing;
      }
    }

    // Generate new short code
    const shortCode = this.generateShortCode();
    const shortenedUrl = new ShortenedUrl(shortCode, originalUrl, new Date());

    // Store in both directions
    this.urls.set(shortCode.value, shortenedUrl);
    this.reverseIndex.set(originalUrl.value, shortCode.value);

    return shortenedUrl;
  }

  retrieveUrl(shortCode: ShortCode): OriginalUrl | null {
    const shortenedUrl = this.urls.get(shortCode.value);
    return shortenedUrl ? shortenedUrl.originalUrl : null;
  }

  private generateShortCode(): ShortCode {
    const chars = 'abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789';
    let result = '';
    
    do {
      result = '';
      for (let i = 0; i < 6; i++) {
        result += chars.charAt(Math.floor(Math.random() * chars.length));
      }
    } while (this.urls.has(result));

    return new ShortCode(result);
  }
}