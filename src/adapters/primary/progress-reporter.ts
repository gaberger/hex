// @ts-nocheck — legacy adapter, replaced by Rust CLI (ADR-010)
/**
 * Progress Reporter — tree-style progress output for CLI operations.
 *
 * Renders progress updates using box-drawing characters when the terminal
 * supports Unicode, falling back to plain ASCII otherwise.
 * ANSI colors are only emitted when stdout is a TTY.
 */

const isTTY = typeof process !== 'undefined' && process.stdout?.isTTY === true;

function formatNum(n: number): string {
  return n.toLocaleString('en-US');
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

export class ProgressReporter {
  constructor(private readonly write: (text: string) => void) {}

  /** Show a header line for the init operation */
  header(message: string): void {
    this.write(message);
  }

  /** Show a tree-style progress update (intermediate step) */
  phase(name: string, detail?: string): void {
    const prefix = '\u251C\u2500'; // ├─
    const line = detail ? `${prefix} ${name}: ${detail}` : `${prefix} ${name}`;
    this.write(line);
  }

  /** Show the final tree-style step */
  phaseFinal(name: string, detail?: string): void {
    const prefix = '\u2514\u2500'; // └─
    const line = detail ? `${prefix} ${name}: ${detail}` : `${prefix} ${name}`;
    this.write(line);
  }

  /** Show a sub-step under the current phase */
  subPhase(name: string, detail?: string): void {
    const prefix = '\u2502  \u2514\u2500'; // │  └─
    const line = detail ? `${prefix} ${name}: ${detail}` : `${prefix} ${name}`;
    this.write(line);
  }

  /** Show a progress bar for file scanning */
  scanning(found: number, excluded: number, _elapsed: number): void {
    this.phase('Scanning filesystem', `${formatNum(found)} files found`);
    if (excluded > 0) {
      this.phase('Applying ignore patterns', `${formatNum(excluded)} files excluded`);
    }
  }

  /** Show indexing progress with a progress bar */
  indexing(current: number, total: number, elapsed: number): void {
    const pct = total > 0 ? Math.round((current / total) * 100) : 0;
    const filled = Math.round(pct / 5); // 20-char bar
    const barFull = '\u2588'; // █
    const barEmpty = '\u2591'; // ░
    const bar = barFull.repeat(filled) + barEmpty.repeat(20 - filled);
    this.phase(
      `Indexing ${formatNum(total)} source files...`,
    );
    this.subPhase(
      `[${bar}] ${pct}% (${formatNum(current)}/${formatNum(total)}) \u2014 ${formatDuration(elapsed)} elapsed`,
    );
  }

  /** Show completion summary */
  complete(stats: { files: number; excluded: number; duration: number }): void {
    this.phaseFinal(
      'Complete',
      `${formatNum(stats.files)} files indexed in ${formatDuration(stats.duration)}`,
    );
  }

  /** Show a mode announcement */
  mode(name: string, detail: string): void {
    this.phase(name, detail);
  }

  /** Show a skip notice */
  skipped(name: string, reason: string): void {
    this.phase(name, `skipped (${reason})`);
  }
}
