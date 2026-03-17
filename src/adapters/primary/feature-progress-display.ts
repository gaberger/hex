/**
 * Feature Progress Display (Primary Adapter)
 *
 * Renders a clean, persistent progress view for feature development.
 * Replaces noisy agent logs with a structured status display.
 * Handles keyboard input for interactive controls (d/q/h).
 */

import type { IFeatureProgressPort, FeatureSession, Workplan } from '../../core/ports/feature-progress.js';
import type { ProgressReport } from '../../core/ports/notification.js';

// ─── ANSI Helpers ────────────────────────────────────────────

const ANSI = {
  reset: '\x1b[0m',
  bold: '\x1b[1m',
  dim: '\x1b[2m',
  red: '\x1b[31m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  cyan: '\x1b[36m',
  gray: '\x1b[90m',
  clearScreen: '\x1b[2J\x1b[H',
  clearLine: '\x1b[2K',
  cursorUp: (n: number) => `\x1b[${n}A`,
  hideCursor: '\x1b[?25l',
  showCursor: '\x1b[?25h',
} as const;

function colorize(text: string, ...codes: string[]): string {
  return codes.join('') + text + ANSI.reset;
}

const STATUS_ICON_ANSI: Record<string, string> = {
  done: colorize('\u2713', ANSI.green),      // ✓
  running: colorize('\u27F3', ANSI.cyan),    // ⟳
  failed: colorize('\u2717', ANSI.red),      // ✗
  queued: colorize('\u23F3', ANSI.gray),     // ⏳
  blocked: colorize('\u26A0', ANSI.yellow),  // ⚠
};

// ─── Progress Bar ────────────────────────────────────────────

function progressBarAnsi(percent: number, width: number): string {
  const filled = Math.round((percent / 100) * width);
  const empty = width - filled;
  const block = '\u2588'; // █
  const light = '\u2591'; // ░
  return (
    colorize(block.repeat(filled), ANSI.green) +
    colorize(light.repeat(empty), ANSI.dim)
  );
}

function padRight(str: string, len: number): string {
  return str.length >= len ? str : str + ' '.repeat(len - str.length);
}

function formatDuration(ms: number): string {
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const secs = seconds % 60;
  return `${minutes}m${secs.toString().padStart(2, '0')}s`;
}

function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(1)}k`;
  return `${tokens}`;
}

// ─── Display Adapter ─────────────────────────────────────────

export class FeatureProgressDisplay {
  private clearLines = 0;
  private detailMode = false;
  private isTTY = process.stdout.isTTY ?? false;
  private keyHandler: ((data: Buffer) => void) | null = null;

  constructor(
    private readonly featureProgress: IFeatureProgressPort,
    private readonly verbose: boolean = false,
  ) {}

  /**
   * Start the display. Shows initial state and subscribes to progress updates.
   * Blocks until feature completes (or user aborts with 'q').
   */
  async start(featureName: string): Promise<void> {
    const session = await this.featureProgress.startFeature(featureName);

    if (!this.verbose && this.isTTY) {
      // Hide cursor for clean rendering
      process.stdout.write(ANSI.hideCursor);

      // Subscribe to progress updates
      this.featureProgress.onProgress((report) => {
        this.render(report, session);
      });

      // Setup keyboard handler
      this.setupKeyboard();

      // Initial render
      const report = await this.featureProgress.getProgress();
      this.render(report, session);
    } else {
      // Verbose mode: just print milestones
      this.featureProgress.onProgress((report) => {
        console.log(`[${report.phase}] ${report.overallPercent}% complete`);
      });
    }
  }

  /**
   * Stop the display and restore terminal state.
   */
  stop(): void {
    if (this.isTTY) {
      process.stdout.write(ANSI.showCursor);
    }
    if (this.keyHandler && process.stdin.isTTY) {
      process.stdin.setRawMode(false);
      process.stdin.off('data', this.keyHandler);
      this.keyHandler = null;
    }
  }

  // ─── Rendering ───────────────────────────────────────────

  private render(report: ProgressReport, session: FeatureSession): void {
    this.clear();

    const lines = [
      ...this.renderHeader(session),
      ...this.renderPhases(session),
      '',
      ...this.renderWorkplan(report, session),
      '',
      ...this.renderSummary(report, session),
      '',
      ...this.renderBlockers(report),
      '',
      ...this.renderFooter(),
    ];

    console.log(lines.join('\n'));
    this.clearLines = lines.length;
  }

  private clear(): void {
    if (!this.isTTY || this.verbose) return;
    if (this.clearLines > 0) {
      // Move cursor up N lines and clear to end of screen
      process.stdout.write(ANSI.cursorUp(this.clearLines) + '\x1b[J');
    }
  }

  private renderHeader(session: FeatureSession): string[] {
    const divider = '\u2500'.repeat(70); // ─
    return [
      colorize(`hex feature: ${session.featureName}`, ANSI.bold, ANSI.white),
      colorize(divider, ANSI.dim),
    ];
  }

  private renderPhases(session: FeatureSession): string[] {
    const lines: string[] = [];
    const phaseLabels: Record<string, string> = {
      init: 'INIT',
      specs: 'SPECS',
      plan: 'PLAN',
      worktrees: 'WORKTREES',
      'tier-0': 'TIER-0',
      'tier-1': 'TIER-1',
      'tier-2': 'TIER-2',
      'tier-3': 'TIER-3',
      validate: 'VALIDATE',
      integrate: 'INTEGRATE',
      finalize: 'FINALIZE',
    };

    for (let i = 0; i < session.phases.length; i++) {
      const phase = session.phases[i];
      const icon =
        phase.status === 'done'
          ? STATUS_ICON_ANSI.done
          : phase.status === 'in-progress'
            ? STATUS_ICON_ANSI.running
            : phase.status === 'failed'
              ? STATUS_ICON_ANSI.failed
              : STATUS_ICON_ANSI.queued;

      const label = padRight(phaseLabels[phase.phase] ?? phase.phase.toUpperCase(), 12);
      const statusText = phase.output ? `(${phase.output})` : '';

      lines.push(`Phase ${i + 1}/11: ${label} ${icon} ${colorize(statusText, ANSI.dim)}`);
    }

    return lines;
  }

  private renderWorkplan(report: ProgressReport, session: FeatureSession): string[] {
    const lines: string[] = [colorize('Workplan:', ANSI.bold)];

    if (!session.workplan) {
      lines.push(colorize('  (pending — plan phase not yet complete)', ANSI.dim));
      return lines;
    }

    const tiers = this.groupByTier(session.workplan);

    for (const tier of tiers) {
      lines.push('');
      lines.push(`  ${colorize(`Tier ${tier.level}`, ANSI.bold)} (${tier.label})`);

      for (const task of tier.tasks) {
        const agent = report.agents.find((a) => a.agentName === task.id);
        const status = agent?.status ?? 'queued';
        const icon = STATUS_ICON_ANSI[status];
        const percent = this.agentPercent(agent);
        const bar = progressBarAnsi(percent, 12);
        const name = padRight(this.shortAdapterName(task.adapter), 20);
        const step = agent?.currentStep ? padRight(agent.currentStep, 8) : '';
        const quality = agent?.qualityScore ? colorize(`Q:${agent.qualityScore}`, ANSI.cyan) : '';

        lines.push(`    ${icon} ${name} ${bar} ${step} ${quality}`);
      }
    }

    return lines;
  }

  private renderSummary(report: ProgressReport, session: FeatureSession): string[] {
    const elapsed = Date.now() - session.startedAt;
    const doneCount = report.agents.filter((a) => a.status === 'done').length;
    const totalCount = report.agents.length;

    const parts = [
      `Overall: ${report.overallPercent}%`,
      `Tasks: ${doneCount}/${totalCount}`,
      `Tokens: ${formatTokens(session.tokenUsed)}/${formatTokens(session.tokenBudget)}`,
      `Time: ${formatDuration(elapsed)}`,
      `Blockers: ${report.blockers.length}`,
    ];

    return [parts.join(' │ ')];
  }

  private renderBlockers(report: ProgressReport): string[] {
    if (report.blockers.length === 0) {
      return [colorize('Blockers: none', ANSI.green)];
    }

    const lines = [colorize('Blockers:', ANSI.yellow, ANSI.bold)];
    for (const blocker of report.blockers) {
      const elapsed = formatDuration(Date.now() - blocker.since);
      lines.push(
        `  ${STATUS_ICON_ANSI.blocked} ${blocker.agentName}: ${blocker.description} (${elapsed})`,
      );
      if (blocker.suggestedAction) {
        lines.push(colorize(`     → ${blocker.suggestedAction}`, ANSI.dim));
      }
    }
    return lines;
  }

  private renderFooter(): string[] {
    const divider = colorize('\u2500'.repeat(70), ANSI.dim);
    const help = colorize('[Press d=details | q=abort | h=help]', ANSI.dim);
    return [divider, help];
  }

  // ─── Keyboard Input ──────────────────────────────────────

  private setupKeyboard(): void {
    if (!process.stdin.isTTY) return;

    process.stdin.setRawMode(true);
    process.stdin.resume();
    process.stdin.setEncoding('utf8');

    this.keyHandler = (data: Buffer) => {
      const char = data.toString();
      if (char === 'd') this.toggleDetails();
      if (char === 'q') this.abort();
      if (char === 'h') this.showHelp();
      if (char === '\u0003') {
        // Ctrl+C
        this.abort();
      }
    };

    process.stdin.on('data', this.keyHandler);
  }

  private toggleDetails(): void {
    this.detailMode = !this.detailMode;
    if (this.detailMode) {
      console.log(colorize('\nDetail mode: showing agent logs...', ANSI.cyan));
      console.log(colorize('Agent logs are in .hex/logs/agent-<name>.log', ANSI.dim));
      console.log(colorize('Press d again to return to progress view\n', ANSI.dim));
    }
  }

  private abort(): void {
    console.log(colorize('\n\nAborting feature development...', ANSI.yellow));
    this.stop();
    process.exit(0);
  }

  private showHelp(): void {
    console.log(colorize('\n\nHex Feature Development Help', ANSI.bold));
    console.log(colorize('\nKeyboard shortcuts:', ANSI.bold));
    console.log('  d - Toggle detail mode (show agent log paths)');
    console.log('  q - Abort feature development (no cleanup)');
    console.log('  h - Show this help');
    console.log('  Ctrl+C - Same as q\n');
    console.log(colorize('Status icons:', ANSI.bold));
    console.log(`  ${STATUS_ICON_ANSI.done} - Task complete`);
    console.log(`  ${STATUS_ICON_ANSI.running} - Task in progress`);
    console.log(`  ${STATUS_ICON_ANSI.queued} - Task queued`);
    console.log(`  ${STATUS_ICON_ANSI.blocked} - Task blocked`);
    console.log(`  ${STATUS_ICON_ANSI.failed} - Task failed`);
    console.log(colorize('\nPress any key to return to progress view...', ANSI.dim));
  }

  // ─── Helpers ─────────────────────────────────────────────

  private groupByTier(workplan: Workplan): Array<{
    level: number;
    label: string;
    tasks: typeof workplan.steps;
  }> {
    const tiers = new Map<number, typeof workplan.steps>();

    for (const step of workplan.steps) {
      if (!tiers.has(step.tier)) {
        tiers.set(step.tier, []);
      }
      tiers.get(step.tier)!.push(step);
    }

    const tierLabels: Record<number, string> = {
      0: 'Domain & Ports',
      1: 'Secondary Adapters',
      2: 'Primary Adapters',
      3: 'Use Cases & Composition Root',
      4: 'Integration Tests',
    };

    return Array.from(tiers.entries())
      .sort(([a], [b]) => a - b)
      .map(([level, tasks]) => ({
        level,
        label: tierLabels[level] ?? `Tier ${level}`,
        tasks,
      }));
  }

  private shortAdapterName(adapter: string): string {
    // "secondary/git-adapter" → "git-adapter"
    const parts = adapter.split('/');
    return parts[parts.length - 1];
  }

  private agentPercent(agent?: typeof ProgressReport.prototype.agents[number]): number {
    if (!agent) return 0;
    if (agent.status === 'done') return 100;
    if (agent.status === 'queued') return 0;
    // Running/blocked: show iteration progress up to 90% (never 100 until done)
    return Math.min(90, (agent.iteration / agent.maxIterations) * 90);
  }
}
