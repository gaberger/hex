import { URL } from '../domain/URL.js';
import { URLId } from '../domain/URLId.js';

export interface URLRepository {
  save(url: URL): Promise<void>;
  findById(id: URLId): Promise<URL | null>;
  findByOriginalUrl(originalUrl: string): Promise<URL | null>;
  delete(id: URLId): Promise<void>;
  exists(id: URLId): Promise<boolean>;
}