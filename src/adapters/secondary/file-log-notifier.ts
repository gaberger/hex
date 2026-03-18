/**
 * File Log Notifier (Secondary / Driven)
 *
 * Implements {@link INotificationEmitPort} for structured JSONL logging
 * to `.hex/activity.log`. Rotates at 10 MB. Includes full context
 * (quality scores, token usage, files changed) so the log is queryable
 * by the {@link NotificationQueryAdapter}.
 */

import type {
  INotificationEmitPort,
  Notification,
  NotificationChannel,
  StatusLine,
  DecisionRequest,
  DecisionResponse,
  ProgressReport,
} from '../../core/ports/notification.js';

/** Abstraction over fs operations for testability. */
interface FileSystem {
  appendFile(path: string, data: string): Promise<void>;
  rename(oldPath: string, newPath: string): Promise<void>;
  stat(path: string): Promise<{ size: number }>;
  mkdir(path: string, opts?: { recursive: boolean }): Promise<void>;
  readFile(path: string, encoding: string): Promise<string>;
}

/** JSONL log entry written to disk. */
interface LogEntry {
  ts: number;
  id: string;
  level: string;
  agent: string;
  agentType: string;
  phase: string;
  adapter?: string;
  title: string;
  detail?: string;
  context?: Record<string, unknown>;
  type: 'notification' | 'status' | 'decision_request' | 'decision_response' | 'progress';
}

const MAX_SIZE_BYTES = 10 * 1024 * 1024; // 10 MB
const DEFAULT_LOG_DIR = '.hex';
const DEFAULT_LOG_FILE = 'activity.log';

/**
 * Append-only JSONL logger with size-based rotation.
 *
 * Inject a {@link FileSystem} for testing without touching real disk.
 */
export class FileLogNotifier implements INotificationEmitPort {
  private readonly logDir: string;
  private readonly logFile: string;
  private currentSize = 0;
  private initialized = false;

  constructor(
    private readonly fs: FileSystem,
    options?: { logDir?: string; logFile?: string },
  ) {
    this.logDir = options?.logDir ?? DEFAULT_LOG_DIR;
    this.logFile = options?.logFile ?? DEFAULT_LOG_FILE;
  }

  /** Write a notification as a JSONL entry. */
  async notify(notification: Omit<Notification, 'id' | 'timestamp'>): Promise<void> {
    const entry: LogEntry = {
      ts: Date.now(),
      id: crypto.randomUUID(),
      level: notification.level,
      agent: notification.source.agentName,
      agentType: notification.source.agentType,
      phase: notification.source.phase,
      adapter: notification.source.adapter,
      title: notification.title,
      detail: notification.detail,
      context: notification.context as Record<string, unknown> | undefined,
      type: 'notification',
    };
    await this.appendEntry(entry);
  }

  /** Log status line updates (compact form only). */
  async updateStatusLine(status: StatusLine): Promise<void> {
    const entry: LogEntry = {
      ts: Date.now(),
      id: crypto.randomUUID(),
      level: 'trace',
      agent: 'system',
      agentType: 'status',
      phase: 'execute',
      title: status.compact,
      type: 'status',
    };
    await this.appendEntry(entry);
  }

  /** Log a decision request; return auto-timeout response (file logger cannot prompt). */
  async requestDecision(request: Omit<DecisionRequest, 'id'>): Promise<DecisionResponse> {
    const id = crypto.randomUUID();
    const entry: LogEntry = {
      ts: Date.now(),
      id,
      level: 'decision',
      agent: request.agentName,
      agentType: 'decision',
      phase: 'execute',
      title: request.question,
      detail: request.options.map((o) => `${o.id}: ${o.label}`).join('; '),
      type: 'decision_request',
    };
    await this.appendEntry(entry);

    const defaultOpt = request.defaultOption ?? request.options[0]?.id ?? '';
    const response: DecisionResponse = {
      requestId: id,
      selectedOption: defaultOpt,
      respondedBy: 'auto_timeout',
      timestamp: Date.now(),
    };

    await this.appendEntry({
      ...entry,
      id: crypto.randomUUID(),
      title: `Decision auto-resolved: ${defaultOpt}`,
      type: 'decision_response',
    });

    return response;
  }

  /** Log a full progress report snapshot. */
  async reportProgress(report: ProgressReport): Promise<void> {
    const entry: LogEntry = {
      ts: Date.now(),
      id: crypto.randomUUID(),
      level: 'info',
      agent: 'swarm',
      agentType: 'coordinator',
      phase: report.phase,
      title: `Progress: ${report.overallPercent}%`,
      context: {
        agents: report.agents.length,
        blockers: report.blockers.length,
        estimatedRemaining: report.estimatedRemaining,
      },
      type: 'progress',
    };
    await this.appendEntry(entry);
  }

  /** No-op -- file logging is always available. */
  async registerChannel(
    _channel: NotificationChannel,
    _config?: Record<string, unknown>,
  ): Promise<void> {}

  // ── Query support ────────────────────────────────────────

  /** Read and parse all entries (used by NotificationQueryAdapter). */
  async readEntries(): Promise<LogEntry[]> {
    try {
      const raw = await this.fs.readFile(this.logPath, 'utf-8');
      return raw
        .split('\n')
        .filter(Boolean)
        .map((line) => JSON.parse(line) as LogEntry);
    } catch {
      // Log file may not exist or contain corrupted lines — return empty
      return [];
    }
  }

  // ── Internal ─────────────────────────────────────────────

  private get logPath(): string {
    return `${this.logDir}/${this.logFile}`;
  }

  private async ensureDir(): Promise<void> {
    if (!this.initialized) {
      await this.fs.mkdir(this.logDir, { recursive: true });
      try {
        const stat = await this.fs.stat(this.logPath);
        this.currentSize = stat.size;
      } catch {
        this.currentSize = 0;
      }
      this.initialized = true;
    }
  }

  private async appendEntry(entry: LogEntry): Promise<void> {
    await this.ensureDir();
    const line = JSON.stringify(entry) + '\n';
    const lineBytes = Buffer.byteLength(line, 'utf-8');

    if (this.currentSize + lineBytes > MAX_SIZE_BYTES) {
      await this.rotate();
    }

    await this.fs.appendFile(this.logPath, line);
    this.currentSize += lineBytes;
  }

  private async rotate(): Promise<void> {
    const rotated = `${this.logDir}/${this.logFile}.${Date.now()}.bak`;
    try {
      await this.fs.rename(this.logPath, rotated);
    } catch {
      // File may not exist yet; ignore.
    }
    this.currentSize = 0;
  }
}
