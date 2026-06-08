/**
 * Secondary adapter — wall-clock time source.
 *
 * Adapters import ports only.
 */

import type { IClock } from '../../core/ports/clock.js';

export class SystemClock implements IClock {
  now(): number {
    return Date.now();
  }
}
