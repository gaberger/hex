import { IUrlRepository } from '../../core/ports/url-repository.js';
import { ShortenedUrl } from '../../core/domain/shortened-url.js';

/** Secondary adapter: in-memory persistence for shortened URLs. */
export class InMemoryUrlRepository implements IUrlRepository {
  private readonly entries = new Map<string, ShortenedUrl>();

  save(entry: ShortenedUrl): void {
    this.entries.set(entry.code, entry);
  }

  findByCode(code: string): ShortenedUrl | undefined {
    return this.entries.get(code);
  }

  count(): number {
    return this.entries.size;
  }
}
