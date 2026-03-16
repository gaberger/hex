/**
 * Domain Event Bus Port
 *
 * Central event infrastructure for hex's own dogfooding.
 * All domain events flow through this port — adapters subscribe
 * to react (notifications, logging, metrics) without coupling
 * to the domain core.
 *
 * This is how hex eats its own dogfood: the notification
 * system doesn't import domain entities directly — it subscribes
 * to domain events through this port interface.
 */

import type { DomainEvent } from '../domain/entities.js';

// Re-export so adapters import DomainEvent from ports, not domain
export type { DomainEvent } from '../domain/entities.js';

// ─── Event Handler Type ──────────────────────────────────

export type EventHandler<T extends DomainEvent['type'] = DomainEvent['type']> = (
  event: Extract<DomainEvent, { type: T }>
) => void | Promise<void>;

// ─── Event Filter ────────────────────────────────────────

export interface EventFilter {
  types?: DomainEvent['type'][];
  source?: string;        // Filter by agent name
  adapter?: string;       // Filter by adapter boundary
  minSeverity?: 'info' | 'warning' | 'error';
}

// ─── Subscription Handle ─────────────────────────────────

export interface Subscription {
  id: string;
  unsubscribe(): void;
}

// ─── Output Port (Secondary / Driven) ────────────────────

export interface IEventBusPort {
  /** Publish a domain event to all subscribers */
  publish(event: DomainEvent): Promise<void>;

  /** Subscribe to specific event types */
  subscribe<T extends DomainEvent['type']>(
    eventType: T,
    handler: EventHandler<T>,
  ): Subscription;

  /** Subscribe with a filter */
  subscribeFiltered(
    filter: EventFilter,
    handler: (event: DomainEvent) => void | Promise<void>,
  ): Subscription;

  /** Subscribe to all events (for logging/auditing) */
  subscribeAll(
    handler: (event: DomainEvent) => void | Promise<void>,
  ): Subscription;

  /** Get event history (for replay/debugging) */
  getHistory(filter?: EventFilter, limit?: number): Promise<DomainEvent[]>;

  /** Clear all subscriptions (for testing) */
  reset(): void;
}
