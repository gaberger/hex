export interface URLShortenerPort {
  shortenURL(originalURL: string): Promise<string>;
  expandURL(shortCode: string): Promise<string | null>;
  deleteShortURL(shortCode: string): Promise<boolean>;
  getURLStats(shortCode: string): Promise<URLStats | null>;
}

export interface URLStats {
  shortCode: string;
  originalURL: string;
  clickCount: number;
  createdAt: Date;
  lastAccessedAt: Date | null;
}