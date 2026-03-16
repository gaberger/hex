/**
 * Notification Query Adapter (Primary / Driving)
 *
 * Exposes CLI commands (`hex status`, `hex decisions`, `hex log`) and a
 * polling endpoint for UIs to query progress, pending decisions, and
 * recent notifications. Delegates emission to the injected emit port.
 */

import type {
  INotificationQueryPort,
  INotificationEmitPort,
  Notification,
  NotificationLevel,
  NotificationListener,
  NotificationPreferences,
  ProgressReport,
  DecisionRequest,
  DecisionResponse,
} from '../../core/ports/notification.js';

/** Filter criteria for querying stored notifications. */
export interface NotificationFilter {
  adapter?: string;
  agentName?: string;
  minLevel?: NotificationLevel;
  since?: number;
}

const LEVEL_ORDER: NotificationLevel[] = [
  'trace', 'info', 'success', 'warning', 'error', 'decision', 'milestone',
];

/**
 * Primary adapter that fronts the notification subsystem.
 *
 * It maintains an in-memory ring buffer of recent notifications and
 * delegates real-time output to one or more {@link INotificationEmitPort}
 * implementations (terminal, file, webhook, event bus).
 */
export class NotificationQueryAdapter implements INotificationQueryPort {
  private notifications: (Notification & { id: string; timestamp: number })[] = [];
  private pendingDecisions: Map<string, DecisionRequest> = new Map();
  private preferences: NotificationPreferences;
  private progress: ProgressReport | null = null;
  private readonly maxBuffer: number;
  private readonly listeners: NotificationListener[] = [];

  constructor(
    private readonly emitPort: INotificationEmitPort,
    options?: { maxBuffer?: number; preferences?: Partial<NotificationPreferences> },
  ) {
    this.maxBuffer = options?.maxBuffer ?? 500;
    this.preferences = {
      channels: ['terminal', 'file_log'],
      minLevel: 'info',
      quietMode: false,
      progressInterval: 5000,
      decisionTimeout: 30000,
      groupByAdapter: false,
      showTokenUsage: false,
      ...options?.preferences,
    };
  }

  // ── INotificationQueryPort ───────────────────────────────

  /** Return the latest progress snapshot for all agents. */
  async getProgress(): Promise<ProgressReport> {
    return this.progress ?? {
      swarmId: '',
      phase: 'idle',
      agents: [],
      overallPercent: 0,
      blockers: [],
    };
  }

  /** Return decisions that have not yet been answered. */
  async getPendingDecisions(): Promise<DecisionRequest[]> {
    return Array.from(this.pendingDecisions.values());
  }

  /** Forward a human (or auto-timeout) response to the emit port. */
  async respondToDecision(response: DecisionResponse): Promise<void> {
    this.pendingDecisions.delete(response.requestId);
    // The emit port may relay the response to the waiting agent.
    await this.emitPort.notify({
      level: 'decision',
      source: { agentName: 'human', agentType: 'developer', phase: 'execute' },
      title: `Decision ${response.requestId} answered`,
      detail: `Selected: ${response.selectedOption} by ${response.respondedBy}`,
    });
  }

  /** Return recent notifications, optionally filtered by minimum level. */
  async getRecent(limit: number, minLevel?: NotificationLevel): Promise<Notification[]> {
    const minIdx = minLevel ? LEVEL_ORDER.indexOf(minLevel) : 0;
    return this.notifications
      .filter((n) => LEVEL_ORDER.indexOf(n.level) >= minIdx)
      .slice(-limit);
  }

  /** Merge partial preferences into the current set. */
  async setPreferences(prefs: Partial<NotificationPreferences>): Promise<void> {
    this.preferences = { ...this.preferences, ...prefs };
  }

  /** Register a callback for every ingested notification (used by dashboard SSE). */
  addListener(fn: NotificationListener): void {
    this.listeners.push(fn);
  }

  // ── Ingest methods (called by use cases / domain) ────────

  /** Store a notification and forward it through the emit port. */
  async ingest(notification: Omit<Notification, 'id' | 'timestamp'>): Promise<void> {
    const full: Notification & { id: string; timestamp: number } = {
      ...notification,
      id: crypto.randomUUID(),
      timestamp: Date.now(),
    };
    this.pushToBuffer(full);
    await this.emitPort.notify(notification);

    for (const fn of this.listeners) {
      try { fn(full); } catch { /* listener errors must not break ingestion */ }
    }
  }

  /** Register a pending decision so UIs can discover it. */
  async ingestDecision(request: DecisionRequest): Promise<void> {
    this.pendingDecisions.set(request.id, request);
  }

  /** Update the cached progress report and forward to emit port. */
  async ingestProgress(report: ProgressReport): Promise<void> {
    this.progress = report;
    await this.emitPort.reportProgress(report);
  }

  // ── Query helpers (used by CLI commands) ─────────────────

  /** Filter notifications by adapter, agent, or level. */
  async query(filter: NotificationFilter, limit = 50): Promise<Notification[]> {
    const minIdx = filter.minLevel ? LEVEL_ORDER.indexOf(filter.minLevel) : 0;
    return this.notifications
      .filter((n) => {
        if (filter.adapter && n.source.adapter !== filter.adapter) return false;
        if (filter.agentName && n.source.agentName !== filter.agentName) return false;
        if (LEVEL_ORDER.indexOf(n.level) < minIdx) return false;
        if (filter.since && n.timestamp < filter.since) return false;
        return true;
      })
      .slice(-limit);
  }

  /** Return current preferences (useful for CLI `hex config` display). */
  getPreferences(): NotificationPreferences {
    return { ...this.preferences };
  }

  // ── Internal ─────────────────────────────────────────────

  private pushToBuffer(n: Notification & { id: string; timestamp: number }): void {
    this.notifications.push(n);
    if (this.notifications.length > this.maxBuffer) {
      this.notifications = this.notifications.slice(-this.maxBuffer);
    }
  }
}
