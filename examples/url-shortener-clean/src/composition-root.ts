/**
 * Composition Root — the ONLY file that crosses adapter boundaries.
 *
 * This file wires concrete adapters to port interfaces.
 * No other file should import from adapters/ directly.
 */

import { InMemoryUrlRepository } from './adapters/secondary/in-memory-url-repository.js';
import { ShortenUrl } from './core/usecases/shorten-url.js';

export const urlRepository = new InMemoryUrlRepository();
export const shortenUrl = new ShortenUrl(urlRepository);
