/**
 * Secondary adapters — driven by the application.
 *
 * Rule 4: an adapter imports `ports/` only, never another adapter. Rule 5 is
 * the other half: nothing imports this except the composition root.
 */
import { Count, type CounterStore } from '../../core/ports/counter-store.js';

/**
 * A counter store that keeps the count in memory.
 *
 * Swap it for a file or a database by writing another `CounterStore` and
 * changing one line in the composition root. No use case changes.
 */
export class InMemoryCounterStore implements CounterStore {
  private count: Count = Count.zero();

  load(): Count {
    return this.count;
  }

  save(count: Count): void {
    this.count = count;
  }
}
