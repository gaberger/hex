/**
 * Use cases — the application's verbs.
 *
 * Rule 3: `usecases/` imports `domain/` and `ports/` only. It takes the port
 * as a parameter and never chooses which adapter fills it; that choice belongs
 * to the composition root alone.
 */
import type { Count } from '../domain/count.js';
import type { CounterStore } from '../ports/counter-store.js';

/** Advance the count by one and return the new value. */
export function increment(store: CounterStore): Count {
  const next = store.load().next();
  store.save(next);
  return next;
}
