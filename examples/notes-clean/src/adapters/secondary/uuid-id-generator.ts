/**
 * Secondary adapter — random-ish unique id generator.
 *
 * Adapters import ports only.
 */

import type { IIdGenerator } from '../../core/ports/id-generator.js';

export class UuidIdGenerator implements IIdGenerator {
  private counter = 0;

  next(): string {
    this.counter += 1;
    const rand = Math.floor(Math.random() * 1e9).toString(36);
    return `note-${this.counter}-${rand}`;
  }
}
