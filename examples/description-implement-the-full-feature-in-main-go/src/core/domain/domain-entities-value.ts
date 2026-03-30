export interface ShortCode {
  readonly value: string;
}

export interface OriginalUrl {
  readonly value: string;
}

export interface UrlMapping {
  readonly shortCode: ShortCode;
  readonly originalUrl: OriginalUrl;
  readonly createdAt: Date;
}

export function createShortCode(value: string): ShortCode {
  if (!value || value.trim().length === 0) {
    throw new Error('Short code cannot be empty');
  }
  
  if (!/^[a-zA-Z0-9_-]+$/.test(value)) {
    throw new Error('Short code can only contain alphanumeric characters, underscores, and hyphens');
  }
  
  if (value.length < 1 || value.length > 20) {
    throw new Error('Short code must be between 1 and 20 characters');
  }
  
  return { value: value.trim() };
}

export function createOriginalUrl(value: string): OriginalUrl {
  if (!value || value.trim().length === 0) {
    throw new Error('URL cannot be empty');
  }
  
  try {
    new URL(value);
  } catch {
    throw new Error('Invalid URL format');
  }
  
  return { value: value.trim() };
}

export function createUrlMapping(shortCode: ShortCode, originalUrl: OriginalUrl): UrlMapping {
  return {
    shortCode,
    originalUrl,
    createdAt: new Date()
  };
}