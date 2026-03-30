import {URLValueObject } from '../domain/url-value-object.js';
import { ShortenedURLValueObject } from '../domain/shortened-url-value-object.js';

export interface URLRepository {
  save(url: URLValueObject): Promise<void>;
  find(id: string): Promise<URLValueObject | null>;
}

export interface ShortenedURLRepository {
  save(shortened: ShortenedURLValueObject): Promise<void>;
  find(id: string): Promise<ShortenedURLValueObject | null>;
}