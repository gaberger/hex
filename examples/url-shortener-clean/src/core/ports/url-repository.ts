import { ShortenedUrl } from '../domain/shortened-url.js';

/** Secondary port: persistence for shortened URLs. */
export interface IUrlRepository {
  save(entry: ShortenedUrl): void;
  findByCode(code: string): ShortenedUrl | undefined;
  count(): number;
}
