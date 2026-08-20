import { generateShortCode } from '../domain/shortened-url.js';
import { IUrlRepository } from '../ports/url-repository.js';

export class ShortenUrl {
  constructor(private readonly repo: IUrlRepository) {}

  shorten(longUrl: string): string {
    const code = generateShortCode(this.repo.count());
    this.repo.save({ code, longUrl });
    return code;
  }

  resolve(code: string): string | undefined {
    return this.repo.findByCode(code)?.longUrl;
  }
}
