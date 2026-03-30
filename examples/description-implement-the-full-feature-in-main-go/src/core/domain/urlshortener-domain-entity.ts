export class UrlShortenerEntity {
  constructor(
    public readonly id: string,
    public readonly originalUrl: string,
    public readonly shortCode: string,
    public readonly createdAt: Date
  ) {}
}