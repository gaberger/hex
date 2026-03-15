/**
 * Webhook Notifier (Secondary / Driven)
 *
 * Implements {@link INotificationEmitPort} for external HTTP webhooks.
 * Supports Slack-compatible JSON payloads, 2-second batching, and
 * exponential-backoff retry (3 attempts).
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

/** Injectable HTTP client for testability. */
export interface HttpClient {
  post(url: string, body: string, headers: Record<string, string>): Promise<{ status: number }>;
}

export interface WebhookConfig {
  url: string;
  minLevel: NotificationLevel;
  slackCompatible: boolean;
  batchMs: number;
  maxRetries: number;
}

interface QueuedPayload {
  text: string;
  level: NotificationLevel;
  ts: number;
}

const LEVEL_ORDER: NotificationLevel[] = [
  'trace', 'info', 'success', 'warning', 'error', 'decision', 'milestone',
];

const LEVEL_EMOJI: Record<NotificationLevel, string> = {
  trace: ':mag:',
  info: ':information_source:',
  success: ':white_check_mark:',
  warning: ':warning:',
  error: ':x:',
  decision: ':question:',
  milestone: ':tada:',
};

const DEFAULT_CONFIG: WebhookConfig = {
  url: '',
  minLevel: 'warning',
  slackCompatible: true,
  batchMs: 2000,
  maxRetries: 3,
};

/**
 * Sends notifications to an external webhook URL.
 *
 * Inject an {@link HttpClient} so tests never hit the network.
 */
export class WebhookNotifier implements INotificationEmitPort {
  private config: WebhookConfig;
  private queue: QueuedPayload[] = [];
  private flushTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(
    private readonly http: HttpClient,
    config?: Partial<WebhookConfig>,
  ) {
    this.config = { ...DEFAULT_CONFIG, ...config };
  }

  /** Queue a notification for batched delivery. */
  async notify(notification: Omit<Notification, 'id' | 'timestamp'>): Promise<void> {
    if (!this.meetsLevel(notification.level)) return;
    const emoji = this.config.slackCompatible ? LEVEL_EMOJI[notification.level] + ' ' : '';
    const text = `${emoji}*[${notification.level.toUpperCase()}]* ${notification.source.agentName}: ${notification.title}`
      + (notification.detail ? `\n>${notification.detail}` : '');
    this.enqueue({ text, level: notification.level, ts: Date.now() });
  }

  /** Status line updates are not sent to webhooks. */
  async updateStatusLine(_status: StatusLine): Promise<void> {}

  /** Post a decision request immediately (not batched). */
  async requestDecision(request: Omit<DecisionRequest, 'id'>): Promise<DecisionResponse> {
    const id = crypto.randomUUID();
    const optionList = request.options
      .map((o, i) => `${i + 1}. ${o.label} _[${o.risk}]_ -- ${o.description}`)
      .join('\n');
    const text = `:question: *Decision needed* (${request.agentName})\n${request.question}\n${optionList}`;

    await this.sendPayload(text);

    const defaultOpt = request.defaultOption ?? request.options[0]?.id ?? '';
    return {
      requestId: id,
      selectedOption: defaultOpt,
      respondedBy: 'auto_timeout',
      timestamp: Date.now(),
    };
  }

  /** Post a progress summary (batched). */
  async reportProgress(report: ProgressReport): Promise<void> {
    if (!this.meetsLevel('info')) return;
    const done = report.agents.filter((a) => a.status === 'done').length;
    const text = `:bar_chart: *Progress ${report.overallPercent}%* | ${done}/${report.agents.length} adapters | phase: ${report.phase}`;
    this.enqueue({ text, level: 'info', ts: Date.now() });
  }

  /** Configure the webhook URL and options at runtime. */
  async registerChannel(
    _channel: NotificationChannel,
    config?: Record<string, unknown>,
  ): Promise<void> {
    if (config?.url) this.config.url = String(config.url);
    if (config?.minLevel) this.config.minLevel = config.minLevel as NotificationLevel;
    if (config?.slackCompatible !== undefined) this.config.slackCompatible = Boolean(config.slackCompatible);
  }

  /** Flush any pending messages immediately (useful in tests / shutdown). */
  async flush(): Promise<void> {
    if (this.flushTimer) {
      clearTimeout(this.flushTimer);
      this.flushTimer = null;
    }
    if (this.queue.length === 0) return;
    const batch = this.queue.splice(0);
    const combined = batch.map((p) => p.text).join('\n---\n');
    await this.sendPayload(combined);
  }

  // ── Internal ─────────────────────────────────────────────

  private meetsLevel(level: NotificationLevel): boolean {
    return LEVEL_ORDER.indexOf(level) >= LEVEL_ORDER.indexOf(this.config.minLevel);
  }

  private enqueue(payload: QueuedPayload): void {
    this.queue.push(payload);
    if (!this.flushTimer) {
      this.flushTimer = setTimeout(() => {
        this.flushTimer = null;
        void this.flush();
      }, this.config.batchMs);
    }
  }

  private async sendPayload(text: string): Promise<void> {
    if (!this.config.url) return;
    const body = this.config.slackCompatible
      ? JSON.stringify({ text })
      : JSON.stringify({ message: text, timestamp: Date.now() });

    let lastError: Error | null = null;
    for (let attempt = 0; attempt < this.config.maxRetries; attempt++) {
      try {
        const res = await this.http.post(this.config.url, body, {
          'Content-Type': 'application/json',
        });
        if (res.status >= 200 && res.status < 300) return;
        lastError = new Error(`Webhook returned ${res.status}`);
      } catch (err) {
        lastError = err instanceof Error ? err : new Error(String(err));
      }
      // Exponential backoff: 1s, 2s, 4s
      if (attempt < this.config.maxRetries - 1) {
        await this.sleep(1000 * Math.pow(2, attempt));
      }
    }
    // Silently drop after max retries -- do not block the agent pipeline.
    if (lastError) {
      // Could emit to a fallback channel in a future iteration.
    }
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }
}
