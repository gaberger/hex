/**
 * Status Formatter
 *
 * Formats ProgressReport data into human-readable status lines in three modes:
 * - Compact: single ~80-char line for persistent status bar display
 * - Expanded: multi-line dashboard for verbose output
 * - JSON: structured payload for webhook delivery
 *
 * Supports ANSI color rendering and plain-text fallback for file logging.
 */

import type {
  ProgressReport,
  AgentProgress,
  StatusLine,
} from '../ports/notification.js';

// ─── ANSI Color Helpers ─────────────────────────────────

const ANSI = {
  reset: '\x1b[0m',
  bold: '\x1b[1m',
  dim: '\x1b[2m',
  red: '\x1b[31m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  magenta: '\x1b[35m',
  cyan: '\x1b[36m',
  white: '\x1b[37m',
  gray: '\x1b[90m',
} as const;

function colorize(text: string, ...codes: string[]): string {
  return codes.join('') + text + ANSI.reset;
}

// ─── Status Icons ───────────────────────────────────────

const STATUS_ICON: Record<AgentProgress['status'], string> = {
  done: '\u2713',     // ✓
  running: '\u27F3',  // ⟳
  failed: '\u2717',   // ✗
  queued: '\u23F3',   // ⏳
  blocked: '\u26A0',  // ⚠
};

const STATUS_ICON_ANSI: Record<AgentProgress['status'], string> = {
  done: colorize('\u2713', ANSI.green),
  running: colorize('\u27F3', ANSI.cyan),
  failed: colorize('\u2717', ANSI.red),
  queued: colorize('\u23F3', ANSI.gray),
  blocked: colorize('\u26A0', ANSI.yellow),
};

// ─── Progress Bar ───────────────────────────────────────

const BLOCK_CHARS = ['\u2591', '\u2592', '\u2593', '\u2588']; // ░ ▒ ▓ █

function progressBar(percent: number, width: number): string {
  const filled = Math.round((percent / 100) * width);
  const empty = width - filled;
  return BLOCK_CHARS[3].repeat(filled) + BLOCK_CHARS[0].repeat(empty);
}

function progressBarAnsi(percent: number, width: number): string {
  const filled = Math.round((percent / 100) * width);
  const empty = width - filled;
  const filledStr = colorize(BLOCK_CHARS[3].repeat(filled), ANSI.green);
  const emptyStr = colorize(BLOCK_CHARS[0].repeat(empty), ANSI.dim);
  return filledStr + emptyStr;
}

// ─── Phase Labels ───────────────────────────────────────

function phaseLabel(phase: string): string {
  const labels: Record<string, string> = {
    plan: 'PLAN',
    execute: 'EXEC',
    integrate: 'INTG',
    package: 'PACK',
  };
  return labels[phase] ?? phase.toUpperCase();
}

// ─── Formatting Utilities ───────────────────────────────

function shortAdapterName(adapter: string): string {
  // "secondary/git-adapter" → "git"
  const parts = adapter.split('/');
  const last = parts[parts.length - 1];
  return last.replace(/-adapter$/, '');
}

function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`;
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(1)}k`;
  return `${tokens}`;
}

function formatDuration(ms: number): string {
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  const secs = seconds % 60;
  return `${minutes}m${secs.toString().padStart(2, '0')}s`;
}

function padRight(str: string, len: number): string {
  return str.length >= len ? str : str + ' '.repeat(len - str.length);
}

// ─── Output Mode ────────────────────────────────────────

export type OutputMode = 'ansi' | 'plain' | 'json';

// ─── Compact Format ─────────────────────────────────────

export function formatCompact(
  report: ProgressReport,
  mode: OutputMode = 'ansi',
): string {
  if (mode === 'json') {
    return JSON.stringify(buildJsonPayload(report));
  }

  const useAnsi = mode === 'ansi';
  const phase = phaseLabel(report.phase);
  const bar = useAnsi
    ? progressBarAnsi(report.overallPercent, 10)
    : progressBar(report.overallPercent, 10);

  // Agent summaries: "cli:lint✓ git:test⟳"
  const agentSummaries = report.agents.map((a) => {
    const name = shortAdapterName(a.adapter);
    const step = a.currentStep;
    const icon = useAnsi ? STATUS_ICON_ANSI[a.status] : STATUS_ICON[a.status];
    return `${name}:${step}${icon}`;
  });

  // Average quality
  const qualityScores = report.agents
    .map((a) => a.qualityScore)
    .filter((q): q is number => q !== undefined);
  const avgQuality = qualityScores.length > 0
    ? Math.round(qualityScores.reduce((a, b) => a + b, 0) / qualityScores.length)
    : '--';

  const doneCount = report.agents.filter((a) => a.status === 'done').length;
  const totalCount = report.agents.length;

  const phaseStr = useAnsi
    ? colorize(`[${phase}]`, ANSI.bold, ANSI.blue)
    : `[${phase}]`;

  const qualityStr = useAnsi
    ? colorize(`Q:${avgQuality}`, ANSI.magenta)
    : `Q:${avgQuality}`;

  const progressStr = `${doneCount}/${totalCount} done`;

  const parts = [
    phaseStr,
    `${bar} ${report.overallPercent}%`,
    agentSummaries.join(' '),
    qualityStr,
    progressStr,
  ];

  return parts.join(' | ');
}

// ─── Expanded Format ────────────────────────────────────

export function formatExpanded(
  report: ProgressReport,
  mode: OutputMode = 'ansi',
  elapsed?: number,
  tokenUsage?: { used: number; budget: number },
): string[] {
  if (mode === 'json') {
    return [JSON.stringify(buildJsonPayload(report), null, 2)];
  }

  const useAnsi = mode === 'ansi';
  const lines: string[] = [];
  const divider = '\u2500'.repeat(49); // ─

  // Header
  if (useAnsi) {
    lines.push(colorize(`hex-intf swarm progress ${divider}`, ANSI.bold, ANSI.white));
  } else {
    lines.push(`hex-intf swarm progress ${divider}`);
  }

  // Phase info
  const phaseNum = phaseNumber(report.phase);
  lines.push(`Phase: ${phaseLabel(report.phase)} (${phaseNum} of 4)`);
  lines.push('');

  // Agent rows
  const nameWidth = Math.max(
    14,
    ...report.agents.map((a) => shortAdapterName(a.adapter).length + 6),
  );

  for (const agent of report.agents) {
    const name = padRight(shortAdapterName(agent.adapter) + '-adapter', nameWidth);
    const agentPercent = agentToPercent(agent);
    const bar = useAnsi
      ? progressBarAnsi(agentPercent, 12)
      : progressBar(agentPercent, 12);
    const step = padRight(agentDisplayStep(agent), 6);
    const quality = agent.qualityScore !== undefined
      ? `Q:${agent.qualityScore}`
      : 'Q:--';
    const iter = agent.iteration === 1
      ? '1 iteration'
      : `${agent.iteration} iterations`;

    const icon = useAnsi ? STATUS_ICON_ANSI[agent.status] : STATUS_ICON[agent.status];

    if (useAnsi) {
      lines.push(`  ${colorize(name, ANSI.white)} ${bar} ${step} ${icon}  ${colorize(quality, ANSI.magenta)}  ${colorize(iter, ANSI.dim)}`);
    } else {
      lines.push(`  ${name} ${bar} ${step} ${icon}  ${quality}  ${iter}`);
    }
  }

  lines.push('');

  // Summary line
  const summaryParts: string[] = [`Overall: ${report.overallPercent}%`];
  if (tokenUsage) {
    summaryParts.push(`Tokens: ${formatTokens(tokenUsage.used)}/${formatTokens(tokenUsage.budget)}`);
  }
  if (elapsed !== undefined) {
    summaryParts.push(`Time: ${formatDuration(elapsed)}`);
  }
  lines.push(summaryParts.join(' | '));

  // Blockers
  if (report.blockers.length === 0) {
    lines.push('Blockers: none');
  } else {
    lines.push('Blockers:');
    for (const blocker of report.blockers) {
      const icon = useAnsi
        ? colorize('\u26A0', ANSI.yellow)
        : '\u26A0';
      lines.push(`  ${icon} ${blocker.agentName}: ${blocker.description}`);
    }
  }

  lines.push(divider);

  return lines;
}

// ─── StatusLine Builder ─────────────────────────────────

export function buildStatusLine(
  report: ProgressReport,
  elapsed?: number,
  tokenUsage?: { used: number; budget: number },
): StatusLine {
  return {
    compact: formatCompact(report, 'plain'),
    expanded: formatExpanded(report, 'plain', elapsed, tokenUsage),
    ansiCompact: formatCompact(report, 'ansi'),
  };
}

// ─── JSON Payload ───────────────────────────────────────

export interface StatusJsonPayload {
  swarmId: string;
  phase: string;
  overallPercent: number;
  agents: Array<{
    name: string;
    adapter: string;
    status: string;
    currentStep: string;
    qualityScore: number | null;
    iteration: number;
  }>;
  blockers: Array<{
    agent: string;
    type: string;
    description: string;
  }>;
}

function buildJsonPayload(report: ProgressReport): StatusJsonPayload {
  return {
    swarmId: report.swarmId,
    phase: report.phase,
    overallPercent: report.overallPercent,
    agents: report.agents.map((a) => ({
      name: a.agentName,
      adapter: a.adapter,
      status: a.status,
      currentStep: a.currentStep,
      qualityScore: a.qualityScore ?? null,
      iteration: a.iteration,
    })),
    blockers: report.blockers.map((b) => ({
      agent: b.agentName,
      type: b.type,
      description: b.description,
    })),
  };
}

// ─── Internal Helpers ───────────────────────────────────

function phaseNumber(phase: string): number {
  const phases = ['plan', 'execute', 'integrate', 'package'];
  const idx = phases.indexOf(phase);
  return idx >= 0 ? idx + 1 : 1;
}

function agentToPercent(agent: AgentProgress): number {
  switch (agent.status) {
    case 'done':
      return 100;
    case 'failed':
      return agent.iteration > 0
        ? Math.min(90, (agent.iteration / agent.maxIterations) * 90)
        : 0;
    case 'queued':
      return 0;
    case 'running':
    case 'blocked':
      return Math.min(90, (agent.iteration / agent.maxIterations) * 90);
  }
}

function agentDisplayStep(agent: AgentProgress): string {
  if (agent.status === 'done') return 'done';
  if (agent.status === 'queued') return 'queued';
  if (agent.status === 'failed') return 'failed';
  return agent.currentStep;
}
