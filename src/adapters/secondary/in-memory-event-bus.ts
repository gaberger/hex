/**
 * In-Memory Event Bus
 *
 * Replaces the NULL_EVENT_BUS stub with a real pub/sub spine.
 * All domain events flow through this bus — adapters subscribe
 * to receive events for broadcasting, logging, or pattern learning.
 *
 * Maintains a ring-buffer history of the last N events for
 * late-joining subscribers (e.g., dashboard reconnects).
 */

import type {
  DomainEvent,
  IEventBusPort,
  EventFilter,
  EventHandler,
  Subscription,
} from '../../core/ports/event-bus.js';

const MAX_HISTORY = 500;
let nextSubId = 1;

interface InternalSubscription {
  id: string;
  handler: (event: DomainEvent) => void | Promise<void>;
  filter?: EventFilter;
  eventType?: string;
}

export class InMemoryEventBus implements IEventBusPort {
  private readonly subscriptions = new Map<string, InternalSubscription>();
  private readonly history: DomainEvent[] = [];

  async publish(event: DomainEvent): Promise<void> {
    // Append to ring buffer
    this.history.push(event);
    if (this.history.length > MAX_HISTORY) {
      this.history.shift();
    }

    // Fan out to all matching subscribers
    for (const sub of this.subscriptions.values()) {
      if (this.matches(sub, event)) {
        try {
          await sub.handler(event);
        } catch {
          // Subscriber errors must not break event delivery
        }
      }
    }
  }

  subscribe<T extends DomainEvent['type']>(
    eventType: T,
    handler: EventHandler<T>,
  ): Subscription {
    const id = `sub-${nextSubId++}`;
    this.subscriptions.set(id, {
      id,
      handler: handler as (event: DomainEvent) => void | Promise<void>,
      eventType,
    });
    return { id, unsubscribe: () => this.subscriptions.delete(id) };
  }

  subscribeFiltered(
    filter: EventFilter,
    handler: (event: DomainEvent) => void | Promise<void>,
  ): Subscription {
    const id = `sub-${nextSubId++}`;
    this.subscriptions.set(id, { id, handler, filter });
    return { id, unsubscribe: () => this.subscriptions.delete(id) };
  }

  subscribeAll(
    handler: (event: DomainEvent) => void | Promise<void>,
  ): Subscription {
    const id = `sub-${nextSubId++}`;
    this.subscriptions.set(id, { id, handler });
    return { id, unsubscribe: () => this.subscriptions.delete(id) };
  }

  async getHistory(
    filter?: EventFilter,
    limit?: number,
  ): Promise<DomainEvent[]> {
    let events: DomainEvent[] = this.history;
    if (filter) {
      events = events.filter((e) => this.matchesFilter(filter, e));
    }
    if (limit) {
      events = events.slice(-limit);
    }
    return events;
  }

  reset(): void {
    this.subscriptions.clear();
    this.history.length = 0;
  }

  // ─── Private Helpers ─────────────────────────────────────

  private matches(sub: InternalSubscription, event: DomainEvent): boolean {
    // subscribeAll — no filter, no eventType
    if (!sub.filter && !sub.eventType) return true;
    // subscribe(eventType) — match on type
    if (sub.eventType) return event.type === sub.eventType;
    // subscribeFiltered — match on filter
    if (sub.filter) return this.matchesFilter(sub.filter, event);
    return true;
  }

  private matchesFilter(filter: EventFilter, event: DomainEvent): boolean {
    if (filter.types && !filter.types.includes(event.type)) return false;
    // source and adapter filtering would require DomainEvent to carry
    // those fields — currently they don't, so we skip those filters
    return true;
  }
}
