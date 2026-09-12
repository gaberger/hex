/**
 * Ports — the contracts between layers.
 *
 * Rule 2: `ports/` imports `domain/` only, for value types. A port names
 * *what* is needed, never *how* — so a port mentioning a table, a URL or a
 * file path has already leaked.
 */
import { Count } from '../domain/count.js';

/**
 * Re-exported so an adapter needs to import nothing but this module.
 *
 * Rule 4 is that a secondary adapter imports `ports/` **only** — not
 * `domain/`. Without this line an adapter cannot name the type its own port
 * signature uses, and every implementation reaches past the contract into the
 * domain. The port is the adapter's whole world, so the port hands it over.
 */
export { Count };

/**
 * Somewhere a count can be kept.
 *
 * Note what is absent: no table, no path, no connection string. An in-memory
 * map and a Postgres row satisfy this identically, which is the point.
 */
export interface CounterStore {
  load(): Count;
  save(count: Count): void;
}
