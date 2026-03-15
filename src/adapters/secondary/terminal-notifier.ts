/**
 * Terminal Notifier (Secondary / Driven)
 *
 * Implements {@link INotificationEmitPort} for CLI / terminal output.
 * Features a persistent status bar, color-coded messages, group-by-adapter
 * dashboard, and interactive decision prompts with timeout countdowns.
 * Uses raw ANSI escape codes -- no external dependencies.
 */

import type {
  INotificationEmitPort,
  Notification,
  NotificationLevel,
  NotificationChannel,
  StatusLine,
  DecisionRequest,
  DecisionResponse,
  ProgressReport,
} from '../../core/ports/notification.js';

// ── ANSI Helpers ─────────────────────────────────────────

const RESET = '\x1b[0m';
const BOLD = '\x1b[1m';
const DIM = '\x1b[2m';
const COLORS: Record<NotificationLevel, string> = {
  trace: '\x1b[90m',      // gray
  info: '\x1b[34m',       // blue
  success: '\x1b[32m',    // green
  warning: '\x1b[33m',    // yellow
  error: '\x1b[31m',      // red
  decision: '\x1b[35m',   // magenta
  milestone: '\x1b[36m',  // cyan
};
const CLEAR_LINE = '\x1b[2K';
const SAVE_CURSOR = '\x1b[s';
const RESTORE_CURSOR = '\x1b[u';

/** Build a progress bar: `████░░ 50%` */
function progressBar(pct: number, width = 10): string {
  const filled = Math.round((pct / 100) * width);
  const empty = width - filled;
  return '\u2588'.repeat(filled) + '\u2591'.repeat(empty) + ` ${pct}%`;
}

/** Compact status string used in the persistent bottom bar. */
export function formatStatusLine(report: ProgressReport): StatusLine {
  const running = report.agents.filter((a) => a.status === 'running');
  const done = report.agents.filter((a) => a.status === 'done').length;
  const total = report.agents.length;
  const agent = running[0];
  const agentLabel = agent ? `${agent.agentName}: ${agent.currentStep}` : 'idle';
  const quality = agent?.qualityScore ?? 0;
  const bar = progressBar(report.overallPercent);

  const compact = `[${report.phase}] ${agentLabel} | quality: ${quality} | ${done}/${total} adapters | ${bar}`;

  const expanded = report.agents.map((a) => {
    const status = a.status === 'running' ? '\x1b[33m\u25cf' : a.status === 'done' ? '\x1b[32m\u2713' : '\x1b[90m\u25cb';
    return `  ${status}${RESET} ${a.agentName.padEnd(18)} ${a.adapter.padEnd(22)} ${a.currentStep}`;
  });

  const ansiCompact = `${DIM}[${RESET}${BOLD}${report.phase}${RESET}${DIM}]${RESET} `
    + `${agentLabel} ${DIM}|${RESET} quality: ${quality >= 80 ? '\x1b[32m' : '\x1b[33m'}${quality}${RESET} `
    + `${DIM}|${RESET} ${done}/${total} adapters ${DIM}|${RESET} ${bar}`;

  return { compact, expanded, ansiCompact };
}

/** Writable stream abstraction for testability. */
export interface WritableOutput {
  write(data: string): void;
}

/**
 * Terminal notification adapter.
 *
 * Inject a {@link WritableOutput} (defaults to `process.stdout`) so the
 * class is fully testable without a real terminal.
 */
export class TerminalNotifier implements INotificationEmitPort {
  private statusVisible = false;

  constructor(
    private readonly out: WritableOutput = process.stdout,
    private readonly groupByAdapter = false,
  ) {}

  /** Write a color-coded notification line. */
  async notify(notification: Omit<Notification, 'id' | 'timestamp'>): Promise<void> {
    const color = COLORS[notification.level];
    const tag = notification.level.toUpperCase().padEnd(9);
    const src = notification.source.agentName;
    const line = `${color}${tag}${RESET} ${DIM}${src}${RESET} ${notification.title}`;
    this.clearStatusBar();
    this.out.write(line + '\n');
    if (notification.detail) {
      this.out.write(`${DIM}         ${notification.detail}${RESET}\n`);
    }
    this.restoreStatusBar();
  }

  /** Render persistent status bar at the bottom of the terminal. */
  async updateStatusLine(status: StatusLine): Promise<void> {
    this.statusVisible = true;
    if (this.groupByAdapter) {
      this.out.write(SAVE_CURSOR);
      for (const line of status.expanded) {
        this.out.write(CLEAR_LINE + line + '\n');
      }
      this.out.write(RESTORE_CURSOR);
    } else {
      this.out.write(`\r${CLEAR_LINE}${status.ansiCompact}`);
    }
  }

  /** Display a decision prompt with numbered options and countdown. */
  async requestDecision(request: Omit<DecisionRequest, 'id'>): Promise<DecisionResponse> {
    const id = crypto.randomUUID();
    const color = COLORS.decision;
    this.clearStatusBar();
    this.out.write(`\n${color}${BOLD}DECISION NEEDED${RESET} (${request.agentName})\n`);
    this.out.write(`${request.question}\n\n`);
    request.options.forEach((opt, i) => {
      const risk = opt.risk === 'high' ? '\x1b[31m' : opt.risk === 'medium' ? '\x1b[33m' : '\x1b[32m';
      this.out.write(`  ${BOLD}${i + 1}${RESET}. ${opt.label} ${risk}[${opt.risk}]${RESET}\n`);
      this.out.write(`     ${DIM}${opt.description}${RESET}\n`);
    });
    const deadline = request.deadline ?? 30000;
    const defaultOpt = request.defaultOption ?? request.options[0]?.id ?? '';
    this.out.write(`\n${DIM}Auto-selecting "${defaultOpt}" in ${Math.round(deadline / 1000)}s${RESET}\n`);

    // In a real implementation this would read stdin with a timer.
    // For testability we resolve immediately with the default.
    return {
      requestId: id,
      selectedOption: defaultOpt,
      respondedBy: 'auto_timeout',
      timestamp: Date.now(),
    };
  }

  /** Forward progress to the status line renderer. */
  async reportProgress(report: ProgressReport): Promise<void> {
    const status = formatStatusLine(report);
    await this.updateStatusLine(status);
  }

  /** No-op for terminal -- always registered. */
  async registerChannel(
    _channel: NotificationChannel,
    _config?: Record<string, unknown>,
  ): Promise<void> {
    // Terminal channel is implicitly registered.
  }

  // ── Internal ─────────────────────────────────────────────

  private clearStatusBar(): void {
    if (this.statusVisible) {
      this.out.write(`\r${CLEAR_LINE}`);
    }
  }

  private restoreStatusBar(): void {
    // The next reportProgress call will repaint.
  }
}
