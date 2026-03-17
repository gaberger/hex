/**
 * CLI formatting utilities for hex.
 *
 * Respects NO_COLOR env var and --no-color flag per https://no-color.org/
 * Also detects non-TTY stdout (piping) and disables color automatically.
 *
 * This is a PRIMARY ADAPTER helper — it must NOT import from other adapters.
 * Zero external dependencies.
 */

// ── Color detection ──

const isColorEnabled = (): boolean => {
  if (process.env['NO_COLOR'] !== undefined) return false;
  if (process.env['FORCE_COLOR'] !== undefined) return true;
  if (!process.stdout.isTTY) return false;
  return true;
};

let _color = isColorEnabled();

/** Call once at CLI startup with --no-color flag state */
export function setColorEnabled(enabled: boolean): void {
  _color = enabled;
}

export function colorEnabled(): boolean {
  return _color;
}

// ── Unicode detection ──

const isUnicodeSupported = (): boolean => {
  if (process.env['HEX_ASCII'] !== undefined) return false;
  if (process.env['FORCE_UNICODE'] !== undefined) return true;
  // Default to ASCII — it renders correctly everywhere.
  // Unicode box-drawing adds minimal value over plain dashes/asterisks,
  // but garbled output is a terrible first impression.
  return false;
};

let _unicode = isUnicodeSupported();

export function setUnicodeEnabled(enabled: boolean): void {
  _unicode = enabled;
}

export function unicodeEnabled(): boolean {
  return _unicode;
}

/** Pick Unicode or ASCII glyph based on terminal capability */
export function glyph(unicode: string, ascii: string): string {
  return _unicode ? unicode : ascii;
}

/** Horizontal rule of given length */
export function hr(len: number): string {
  return glyph('\u2500', '-').repeat(len);
}

// ── ANSI codes ──

const ESC = '\x1b[';
const RESET = `${ESC}0m`;

function wrap(code: string, text: string): string {
  return _color ? `${ESC}${code}m${text}${RESET}` : text;
}

// ── Styles ──

export const bold = (t: string): string => wrap('1', t);
export const dim = (t: string): string => wrap('2', t);
export const italic = (t: string): string => wrap('3', t);
export const underline = (t: string): string => wrap('4', t);

// ── Colors ──

export const red = (t: string): string => wrap('31', t);
export const green = (t: string): string => wrap('32', t);
export const yellow = (t: string): string => wrap('33', t);
export const blue = (t: string): string => wrap('34', t);
export const magenta = (t: string): string => wrap('35', t);
export const cyan = (t: string): string => wrap('36', t);
export const white = (t: string): string => wrap('37', t);
export const gray = (t: string): string => wrap('90', t);

// ── Semantic aliases ──

export const success = (t: string): string => green(t);
export const error = (t: string): string => red(t);
export const warn = (t: string): string => yellow(t);
export const info = (t: string): string => cyan(t);
export const muted = (t: string): string => gray(t);
export const accent = (t: string): string => magenta(t);

// ── Structured output helpers ──

/** hex brand header */
export function header(): string {
  const icon = glyph('\u2B21', '*');
  const dash = glyph('\u2014', '--');
  return `${bold(accent(icon))}  ${bold('hex')} ${muted(`${dash} Hexagonal Architecture Framework`)}`;
}

/** Section divider with title */
export function section(title: string): string {
  const hr = glyph('\u2500', '-');
  const line = hr.repeat(Math.max(0, 50 - title.length - 2));
  return `\n${bold(title)} ${muted(line)}`;
}

/** A labeled key: value line */
export function kv(key: string, value: string, indent = 0): string {
  const pad = ' '.repeat(indent);
  return `${pad}${muted(key + ':')} ${value}`;
}

/** Success/fail/warn status badge */
export function badge(status: 'pass' | 'fail' | 'warn' | 'info' | 'skip'): string {
  switch (status) {
    case 'pass':
      return green(glyph('\u2713', 'v'));
    case 'fail':
      return red(glyph('\u2717', 'x'));
    case 'warn':
      return yellow(glyph('\u26A0', '!'));
    case 'info':
      return cyan(glyph('\u25CF', '*'));
    case 'skip':
      return gray(glyph('\u25CB', 'o'));
  }
}

/** Progress spinner characters */
export const SPINNER_UNICODE = [
  '\u280B', '\u2819', '\u2839', '\u2838', '\u283C', '\u2834', '\u2826', '\u2827', '\u2807', '\u280F',
];
export const SPINNER_ASCII = ['|', '/', '-', '\\'];
export const SPINNER = isUnicodeSupported() ? SPINNER_UNICODE : SPINNER_ASCII;

/** Simple inline progress: [3/7] description */
export function progress(current: number, total: number, label: string): string {
  const pct = total > 0 ? Math.round((current / total) * 100) : 0;
  return `${muted(`[${current}/${total}]`)} ${label} ${muted(`${pct}%`)}`;
}

/** Health score with color coding */
export function healthScore(score: number): string {
  const s = `${score}/100`;
  if (score >= 80) return green(s);
  if (score >= 50) return yellow(s);
  return red(s);
}

/** Strip ANSI escape codes for length calculation */
function stripAnsi(str: string): string {
  return str.replace(/\x1b\[[0-9;]*m/g, '');
}

/** Box drawing for banners */
export function box(lines: string[], opts?: { accent?: boolean }): string {
  const maxLen = Math.max(...lines.map((l) => stripAnsi(l).length), 40);
  const hrChar = glyph('\u2500', '-');
  const tl = glyph('\u250C', '+');
  const tr = glyph('\u2510', '+');
  const bl = glyph('\u2514', '+');
  const br = glyph('\u2518', '+');
  const vr = glyph('\u2502', '|');
  const hr = hrChar.repeat(maxLen + 2);
  const top = `${tl}${hr}${tr}`;
  const bot = `${bl}${hr}${br}`;
  const padded = lines.map((l) => {
    const visible = stripAnsi(l).length;
    const pad = ' '.repeat(Math.max(0, maxLen - visible));
    return `${vr} ${l}${pad} ${vr}`;
  });
  const color = opts?.accent ? accent : (t: string) => t;
  return [color(top), ...padded.map(color), color(bot)].join('\n');
}

/** "Did you mean?" fuzzy command suggestion */
export function didYouMean(input: string, candidates: string[]): string | null {
  let best: string | null = null;
  let bestDist = Infinity;

  for (const c of candidates) {
    const dist = levenshtein(input.toLowerCase(), c.toLowerCase());
    if (dist < bestDist && dist <= Math.max(2, Math.floor(c.length / 2))) {
      bestDist = dist;
      best = c;
    }
  }
  return best;
}

function levenshtein(a: string, b: string): number {
  const m = a.length;
  const n = b.length;
  const dp: number[][] = Array.from({ length: m + 1 }, (_, i) =>
    Array.from({ length: n + 1 }, (_, j) => (i === 0 ? j : j === 0 ? i : 0)),
  );
  for (let i = 1; i <= m; i++) {
    for (let j = 1; j <= n; j++) {
      dp[i][j] =
        a[i - 1] === b[j - 1]
          ? dp[i - 1][j - 1]
          : 1 + Math.min(dp[i - 1][j], dp[i][j - 1], dp[i - 1][j - 1]);
    }
  }
  return dp[m][n];
}

/** Elapsed time formatted as human-readable */
export function elapsed(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
  const mins = Math.floor(ms / 60000);
  const secs = Math.round((ms % 60000) / 1000);
  return `${mins}m ${secs}s`;
}

/** Table formatter -- takes rows as arrays, auto-pads columns */
export function table(headers: string[], rows: string[][]): string {
  const widths = headers.map((h, i) =>
    Math.max(stripAnsi(h).length, ...rows.map((r) => stripAnsi(r[i] ?? '').length)),
  );

  const formatRow = (cells: string[]) =>
    cells
      .map((c, i) => {
        const pad = ' '.repeat(Math.max(0, widths[i] - stripAnsi(c).length));
        return c + pad;
      })
      .join('  ');

  const headerLine = formatRow(headers.map((h) => bold(h)));
  const separator = widths.map((w) => muted(glyph('\u2500', '-').repeat(w))).join('  ');
  const bodyLines = rows.map((r) => formatRow(r));

  return [headerLine, separator, ...bodyLines].join('\n');
}
