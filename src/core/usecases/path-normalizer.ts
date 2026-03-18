/**
 * Path Normalizer — Pure functions for resolving import paths
 * across TypeScript, Go, and Rust.
 *
 * Each language has different import semantics:
 * - TypeScript: relative paths with .js extensions → resolved to .ts
 * - Go: module paths like "github.com/user/pkg" → kept as-is for external,
 *        relative paths within project resolved normally
 * - Rust: crate paths like "crate::core::ports" → converted to file paths
 */

// Pure string helpers replacing node:path/posix to keep usecases free of Node imports

/** Return the directory portion of a forward-slash path (pure string). */
function dirnamePosix(p: string): string {
  const idx = p.lastIndexOf('/');
  if (idx < 0) return '.';
  if (idx === 0) return '/';
  return p.slice(0, idx);
}

/** Join path segments with '/' and normalise (collapse '..' and '.', remove double slashes). */
function joinPosix(...parts: string[]): string {
  const joined = parts.join('/');
  const segments: string[] = [];
  for (const seg of joined.split('/')) {
    if (seg === '' || seg === '.') continue;
    if (seg === '..' && segments.length > 0 && segments[segments.length - 1] !== '..') {
      segments.pop();
    } else {
      segments.push(seg);
    }
  }
  return segments.join('/') || '.';
}

/**
 * Resolve an import path to a project-relative file path.
 *
 * TypeScript: './foo.js' from 'src/bar.ts' → 'src/foo.ts'
 * Go: "../ports" from 'src/adapters/primary/cli.go' → 'src/ports'
 * Go: "net/http" (stdlib) → 'net/http' (kept as-is)
 * Rust: "crate::core::ports" → 'src/core/ports'
 */
export function resolveImportPath(fromFile: string, importPath: string, goModulePrefix?: string): string {
  const lang = detectLang(fromFile);

  if (lang === 'go') return resolveGoImport(fromFile, importPath, goModulePrefix);
  if (lang === 'rust') return resolveRustImport(importPath, fromFile);
  return resolveTsImport(fromFile, importPath);
}

/**
 * Normalize a path for comparison: strip leading ./, preserve original extension.
 */
export function normalizePath(filePath: string): string {
  let p = filePath;

  // Strip leading ./
  while (p.startsWith('./')) {
    p = p.slice(2);
  }

  const lang = detectLang(p);

  if (lang === 'go') {
    // Go files keep .go extension; no transformation needed
    return p;
  }

  if (lang === 'rust') {
    // Rust files keep .rs extension; no transformation needed
    return p;
  }

  // TypeScript: Replace .js/.jsx extension with .ts/.tsx
  if (p.endsWith('.js')) {
    p = p.slice(0, -3) + '.ts';
  } else if (p.endsWith('.jsx')) {
    p = p.slice(0, -4) + '.tsx';
  } else if (p.endsWith('/')) {
    p = p + 'index.ts';
  } else if (!p.endsWith('.ts') && !p.endsWith('.tsx') && !p.includes(':') && !p.endsWith('.go') && !p.endsWith('.rs')) {
    p = p + '.ts';
  }

  return p;
}

// ── Language-specific resolvers ─────────────────────────────

function resolveTsImport(fromFile: string, importPath: string): string {
  if (!importPath.startsWith('.')) {
    return normalizePath(importPath);
  }
  const dir = dirnamePosix(fromFile);
  const resolved = joinPosix(dir, importPath);
  return normalizePath(resolved);
}

function resolveGoImport(_fromFile: string, importPath: string, modulePrefix?: string): string {
  // Go imports are module paths, not file paths.
  // Relative imports within the same project use relative directory paths.
  // External imports (containing dots like "github.com") are kept as-is.
  if (importPath.startsWith('.')) {
    const dir = dirnamePosix(_fromFile);
    return joinPosix(dir, importPath);
  }
  // Strip Go module prefix to get project-relative path for layer classification
  if (modulePrefix && importPath.startsWith(modulePrefix + '/')) {
    return importPath.slice(modulePrefix.length + 1);
  }
  // External or stdlib import — return as-is for boundary classification
  return importPath;
}

/**
 * Return both possible Rust module file candidates for a base path.
 * Rust modules can be either `path.rs` or `path/mod.rs`.
 * The caller (arch-analyzer) should match against actual files.
 * (Internal helper - not currently used)
 */
export function rustModuleCandidates(basePath: string): string[] {
  return [basePath + '.rs', basePath + '/mod.rs'];
}

/**
 * Strip trailing item-name segment from a Rust path.
 * If the path has 3+ segments and the last segment starts with an uppercase
 * letter, it's an item name (type/function), not a file/module.
 */
function stripRustItemName(segments: string[]): string[] {
  if (segments.length >= 3) {
    const last = segments[segments.length - 1];
    if (last.length > 0 && last[0] >= 'A' && last[0] <= 'Z') {
      return segments.slice(0, -1);
    }
  }
  return segments;
}

function resolveRustImport(importPath: string, fromFile: string): string {
  // Rust crate:: paths map to src/ directory structure
  // "crate::core::ports::IFoo" → "src/core/ports" (strip uppercase item name)
  // "crate::adapters::primary::http_adapter" → "src/adapters/primary/http_adapter" (keep lowercase)
  if (importPath.startsWith('crate::')) {
    const segments = stripRustItemName(importPath.slice(7).split('::'));
    return 'src/' + segments.join('/');
  }

  // "self::foo" — current module (resolve relative to importing file's directory)
  if (importPath.startsWith('self::')) {
    const dir = dirnamePosix(fromFile);
    const segments = importPath.slice(6).split('::');
    return joinPosix(dir, ...segments);
  }

  // "super::foo" — parent module
  if (importPath.startsWith('super::')) {
    return importPath.replace(/::/g, '/');
  }

  // External crate or std — return as-is
  return importPath;
}

// ── Helpers ─────────────────────────────────────────────────

function detectLang(filePath: string): 'typescript' | 'go' | 'rust' | 'unknown' {
  if (filePath.endsWith('.ts') || filePath.endsWith('.tsx') || filePath.endsWith('.js') || filePath.endsWith('.jsx')) return 'typescript';
  if (filePath.endsWith('.go')) return 'go';
  if (filePath.endsWith('.rs')) return 'rust';
  return 'unknown';
}
