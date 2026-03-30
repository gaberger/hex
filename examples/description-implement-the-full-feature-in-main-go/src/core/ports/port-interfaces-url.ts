export interface UrlShorteningPort {
  shortenUrl(originalUrl: string): Promise<string>;
  retrieveUrl(shortCode: string): Promise<string | null>;
}

export interface UrlStoragePort {
  store(shortCode: string, originalUrl: string): Promise<void>;
  retrieve(shortCode: string): Promise<string | null>;
}