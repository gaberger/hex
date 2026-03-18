/**
 * Domain Value Object: Ignore Patterns
 *
 * Provides a pure-domain engine for matching file paths against ignore
 * patterns (`.hexignore`, `.gitignore` fallback, built-in defaults).
 *
 * Zero external dependencies — pattern matching is implemented inline.
 */

// ─── Default Patterns ────────────────────────────────────

/** Built-in ignore patterns that are always applied. */
export const DEFAULT_IGNORE_PATTERNS: string[] = [
  // Build output
  'target/',
  'build/',
  'dist/',
  'out/',
  'bin/',
  'obj/',

  // Package managers
  'node_modules/',
  'vendor/',

  // Python virtualenvs
  '.venv/',
  'venv/',

  // Test / coverage artifacts
  'coverage/',
  'test-results/',
  '.pytest_cache/',
  '__pycache__/',

  // Editor / IDE
  '.vscode/',
  '.idea/',
  '*.swp',

  // OS / misc
  '.DS_Store',
  '*.log',
  '*.tmp',

  // VCS
  '.git/',
];

// ─── Pattern Matching Helpers ────────────────────────────

/**
 * Returns true when `segment` matches `pattern` using basic glob rules:
 *   - `*` matches any sequence of non-separator characters
 *   - all other characters match literally
 */
function globMatch(pattern: string, segment: string): boolean {
  let pi = 0;
  let si = 0;
  let starPi = -1;
  let starSi = -1;

  while (si < segment.length) {
    if (pi < pattern.length && pattern[pi] === '*') {
      starPi = pi;
      starSi = si;
      pi++;
    } else if (pi < pattern.length && pattern[pi] === segment[si]) {
      pi++;
      si++;
    } else if (starPi !== -1) {
      pi = starPi + 1;
      starSi++;
      si = starSi;
    } else {
      return false;
    }
  }

  while (pi < pattern.length && pattern[pi] === '*') {
    pi++;
  }

  return pi === pattern.length;
}

// ─── IgnoreEngine ────────────────────────────────────────

/** Minimal filesystem operations needed by `fromProject`. */
interface IgnoreFS {
  read: (path: string) => Promise<string>;
  exists: (path: string) => Promise<boolean>;
}

/**
 * Determines whether a relative path should be ignored based on a
 * set of patterns.  Supports three pattern forms:
 *
 *  1. **Directory pattern** — ends with `/` (e.g. `node_modules/`).
 *     Matches any path segment that equals the directory name.
 *  2. **Glob pattern** — contains `*` (e.g. `*.log`).
 *     Matched against the basename (last segment) of the path.
 *  3. **Exact name** — matched literally against every path segment
 *     and the full basename.
 */
export class IgnoreEngine {
  private readonly dirPatterns: string[];
  private readonly globPatterns: string[];
  private readonly exactPatterns: string[];

  constructor(patterns: string[]) {
    this.dirPatterns = [];
    this.globPatterns = [];
    this.exactPatterns = [];

    for (const raw of patterns) {
      const p = raw.trim();
      if (p === '' || p.startsWith('#')) continue;

      if (p.endsWith('/')) {
        // Strip trailing slash for segment comparison
        this.dirPatterns.push(p.slice(0, -1));
      } else if (p.includes('*')) {
        this.globPatterns.push(p);
      } else {
        this.exactPatterns.push(p);
      }
    }
  }

  /** Check whether `relativePath` matches any ignore pattern. */
  isIgnored(relativePath: string): boolean {
    // Normalise separators and strip leading ./
    const normalised = relativePath.replace(/\\/g, '/').replace(/^\.\//, '');
    const segments = normalised.split('/');
    const basename = segments[segments.length - 1];

    // 1. Directory patterns — any segment matches the directory name
    for (const dir of this.dirPatterns) {
      for (const seg of segments) {
        if (seg === dir) return true;
      }
    }

    // 2. Glob patterns — matched against the basename
    for (const pat of this.globPatterns) {
      if (globMatch(pat, basename)) return true;
    }

    // 3. Exact patterns — matched against basename and every segment
    for (const exact of this.exactPatterns) {
      if (basename === exact) return true;
      for (const seg of segments) {
        if (seg === exact) return true;
      }
    }

    return false;
  }

  // ─── Factory ────────────────────────────────────────────

  /**
   * Build an `IgnoreEngine` for a project root.
   *
   * Resolution order:
   *  1. `.hexignore` — if present, use its patterns
   *  2. `.gitignore` — fallback when no `.hexignore` exists
   *  3. `DEFAULT_IGNORE_PATTERNS` — always merged in
   */
  static async fromProject(
    rootPath: string,
    fs: IgnoreFS,
  ): Promise<IgnoreEngine> {
    const sep = rootPath.endsWith('/') ? '' : '/';
    let projectPatterns: string[] = [];

    const hexIgnorePath = `${rootPath}${sep}.hexignore`;
    const gitIgnorePath = `${rootPath}${sep}.gitignore`;

    if (await fs.exists(hexIgnorePath)) {
      const content = await fs.read(hexIgnorePath);
      projectPatterns = parseIgnoreFile(content);
    } else if (await fs.exists(gitIgnorePath)) {
      const content = await fs.read(gitIgnorePath);
      projectPatterns = parseIgnoreFile(content);
    }

    // Merge: project-specific patterns first, then defaults
    return new IgnoreEngine([...projectPatterns, ...DEFAULT_IGNORE_PATTERNS]);
  }
}

// ─── File Parsing ────────────────────────────────────────

/** Parse ignore-file content into a list of patterns (strips comments and blanks). */
function parseIgnoreFile(content: string): string[] {
  return content
    .split('\n')
    .map((line) => line.trim())
    .filter((line) => line !== '' && !line.startsWith('#'));
}
