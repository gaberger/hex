export interface Url {
  readonly value: string;
}

export interface UrlId {
  readonly value: string;
}

export interface ShortCode {
  readonly value: string;
}

export interface UrlMetadata {
  readonly title?: string;
  readonly description?: string;
  readonly createdAt: Date;
  readonly accessCount: number;
  readonly lastAccessedAt?: Date;
}

export interface UrlEntity {
  readonly id: UrlId;
  readonly originalUrl: Url;
  readonly shortCode: ShortCode;
  readonly metadata: UrlMetadata;
}

export function createUrl(value: string): Url {
  if (!value || typeof value !== 'string') {
    throw new Error('URL value must be a non-empty string');
  }
  
  try {
    new URL(value);
  } catch {
    throw new Error('Invalid URL format');
  }
  
  return { value };
}

export function createUrlId(value: string): UrlId {
  if (!value || typeof value !== 'string') {
    throw new Error('URL ID must be a non-empty string');
  }
  
  return { value };
}

export function createShortCode(value: string): ShortCode {
  if (!value || typeof value !== 'string') {
    throw new Error('Short code must be a non-empty string');
  }
  
  if (!/^[a-zA-Z0-9_-]+$/.test(value)) {
    throw new Error('Short code can only contain alphanumeric characters, hyphens, and underscores');
  }
  
  return { value };
}

export function createUrlMetadata(options: {
  title?: string;
  description?: string;
  createdAt?: Date;
  accessCount?: number;
  lastAccessedAt?: Date;
}): UrlMetadata {
  return {
    title: options.title,
    description: options.description,
    createdAt: options.createdAt ?? new Date(),
    accessCount: options.accessCount ?? 0,
    lastAccessedAt: options.lastAccessedAt,
  };
}

export function createUrlEntity(options: {
  id: UrlId;
  originalUrl: Url;
  shortCode: ShortCode;
  metadata: UrlMetadata;
}): UrlEntity {
  return {
    id: options.id,
    originalUrl: options.originalUrl,
    shortCode: options.shortCode,
    metadata: options.metadata,
  };
}

export function incrementAccessCount(entity: UrlEntity): UrlEntity {
  return {
    ...entity,
    metadata: {
      ...entity.metadata,
      accessCount: entity.metadata.accessCount + 1,
      lastAccessedAt: new Date(),
    },
  };
}