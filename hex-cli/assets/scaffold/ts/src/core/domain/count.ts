/**
 * Domain — value objects and entities. Pure data, no I/O.
 *
 * Rule 1: `domain/` imports only `domain/`. Nothing here may reach for a port,
 * a use case, or an adapter. That is what makes it testable with no wiring.
 */

/** How many times something has happened. Never negative, by construction. */
export class Count {
  private constructor(private readonly n: number) {}

  static zero(): Count {
    return new Count(0);
  }

  /**
   * The next count. Saturates rather than wrapping: a counter that silently
   * restarts at zero is worse than one that stops.
   */
  next(): Count {
    return this.n === Number.MAX_SAFE_INTEGER ? this : new Count(this.n + 1);
  }

  value(): number {
    return this.n;
  }
}
