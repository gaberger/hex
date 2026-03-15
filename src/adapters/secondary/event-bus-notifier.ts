/**
 * Event Bus Notifier (Secondary / Driven)
 *
 * Implements {@link INotificationEmitPort} for internal agent-to-agent
 * communication via an in-memory pub/sub pattern. Agents subscribe to
 * notifications from other agents for coordination, blocker detection,
 * and swarm-level decision propagation.
 *
 * Supports exact and wildcard subscriptions (e.g. subscribe to all
 * 'error' level events across agents).
 */

import type {
  INotificationEmitPort,
  Notification,
  NotificationChannel,
  NotificationLevel,
  StatusLine,
  DecisionRequest,
  DecisionResponse,
  ProgressReport,
} from '../../core/ports/notification.js';

/** Callback invoked when a matching notification arrives. */
export type NotificationHandler = (notification: Notification) => void;

/** Subscription filter: all fields are optional wildcards. */
export interface SubscriptionFilter {
  agentName?: string;   // Exact agent name, or '*' for all
  level?: NotificationLevel | '*';
  adapter?: string;     // Exact adapter, or '*' for all
}

interface Subscription {
  id: string;
  filter: SubscriptionFilter;
  handler: NotificationHandler;
}

/**
 * In-memory pub/sub notification bus.
 *
 * No external dependencies. Subscriptions are stored in a plain array;
 * this is sufficient for the expected scale (tens of agents, not thousands).
 */
export class EventBusNotifier implements INotificationEmitPort {
  private subscriptions: Subscription[] = [];
  private decisionHandlers: Map<string, (response: DecisionResponse) => void> = new Map();

  /** Emit a notification to all matching subscribers. */
  async notify(notification: Omit<Notification, 'id' | 'timestamp'>): Promise<void> {
    const full: Notification = {
      ...notification,
      id: crypto.randomUUID(),
      timestamp: Date.now(),
    };
    for (const sub of this.subscriptions) {
      if (this.matches(sub.filter, full)) {
        try { sub.handler(full); } catch { /* subscriber errors must not break the bus */ }
      }
    }
  }

  /** Status line updates are forwarded as 'trace' notifications. */
  async updateStatusLine(status: StatusLine): Promise<void> {
    await this.notify({
      level: 'trace',
      source: { agentName: 'system', agentType: 'status', phase: 'execute' },
      title: status.compact,
    });
  }

  /**
   * Broadcast a decision request to subscribers.
   *
   * If an agent has registered a decision handler, it may respond.
   * Otherwise, falls back to auto-timeout with the default option.
   */
  async requestDecision(request: Omit<DecisionRequest, 'id'>): Promise<DecisionResponse> {
    const id = crypto.randomUUID();
    void ({ ...request, id } satisfies DecisionRequest);

    // Broadcast as a 'decision' notification so watchers see it.
    await this.notify({
      level: 'decision',
      source: { agentName: request.agentName, agentType: 'decision', phase: 'execute' },
      title: request.question,
      detail: request.options.map((o) => `${o.id}: ${o.label}`).join(', '),
      actions: request.options.map((o) => ({
        label: o.label,
        type: 'choose' as const,
        payload: { optionId: o.id },
      })),
    });

    // Wait for an agent to respond, or auto-resolve after deadline.
    const deadline = request.deadline ?? 30000;
    return new Promise<DecisionResponse>((resolve) => {
      const timer = setTimeout(() => {
        this.decisionHandlers.delete(id);
        resolve({
          requestId: id,
          selectedOption: request.defaultOption ?? request.options[0]?.id ?? '',
          respondedBy: 'auto_timeout',
          timestamp: Date.now(),
        });
      }, deadline);

      this.decisionHandlers.set(id, (response) => {
        clearTimeout(timer);
        this.decisionHandlers.delete(id);
        resolve(response);
      });
    });
  }

  /** Forward progress as a notification for agent watchers. */
  async reportProgress(report: ProgressReport): Promise<void> {
    await this.notify({
      level: 'info',
      source: { agentName: 'swarm', agentType: 'coordinator', phase: report.phase as 'execute' },
      title: `Progress: ${report.overallPercent}%`,
      context: {
        stepsCompleted: report.agents.filter((a) => a.status === 'done').length,
        stepsTotal: report.agents.length,
        percentComplete: report.overallPercent,
      },
    });
  }

  /** No-op; event bus is always available internally. */
  async registerChannel(
    _channel: NotificationChannel,
    _config?: Record<string, unknown>,
  ): Promise<void> {}

  // ── Public subscription API ──────────────────────────────

  /**
   * Subscribe to notifications matching the given filter.
   * @returns An unsubscribe function.
   */
  subscribe(filter: SubscriptionFilter, handler: NotificationHandler): () => void {
    const id = crypto.randomUUID();
    this.subscriptions.push({ id, filter, handler });
    return () => {
      this.subscriptions = this.subscriptions.filter((s) => s.id !== id);
    };
  }

  /**
   * Respond to a pending decision request by ID.
   * Used by escalation agents or automated decision engines.
   */
  respondToDecision(response: DecisionResponse): void {
    const handler = this.decisionHandlers.get(response.requestId);
    if (handler) handler(response);
  }

  /** Return the current subscriber count (useful for diagnostics). */
  get subscriberCount(): number {
    return this.subscriptions.length;
  }

  // ── Internal ─────────────────────────────────────────────

  private matches(filter: SubscriptionFilter, n: Notification): boolean {
    if (filter.agentName && filter.agentName !== '*' && n.source.agentName !== filter.agentName) {
      return false;
    }
    if (filter.level && filter.level !== '*' && n.level !== filter.level) {
      return false;
    }
    if (filter.adapter && filter.adapter !== '*' && n.source.adapter !== filter.adapter) {
      return false;
    }
    return true;
  }
}
