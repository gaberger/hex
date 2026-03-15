/**
 * Path Normalizer — Pure functions for resolving relative import paths
 * to project-relative .ts paths.
 *
 * Fixes the mismatch between tree-sitter's relative `.js` import paths
 * and the project-relative `.ts` paths from fs.glob().
 */

import { posix } from 'node:path';

/**
 * Resolve a relative import path to a project-relative .ts path.
 * Example: given file 'src/adapters/secondary/git.ts' importing '../../core/ports/index.js'
 * returns 'src/core/ports/index.ts'
 */
export function resolveImportPath(fromFile: string, importPath: string): string {
  // Non-relative imports (bare specifiers) are returned as-is after normalization
  if (!importPath.startsWith('.')) {
    return normalizePath(importPath);
  }

  const dir = posix.dirname(fromFile);
  const resolved = posix.join(dir, importPath);
  return normalizePath(resolved);
}

/**
 * Normalize a path for comparison: strip leading ./, ensure .ts extension.
 */
export function normalizePath(filePath: string): string {
  let p = filePath;

  // Strip leading ./
  while (p.startsWith('./')) {
    p = p.slice(2);
  }

  // Replace .js/.jsx extension with .ts/.tsx
  if (p.endsWith('.js')) {
    p = p.slice(0, -3) + '.ts';
  } else if (p.endsWith('.jsx')) {
    p = p.slice(0, -4) + '.tsx';
  } else if (!p.endsWith('.ts') && !p.endsWith('.tsx')) {
    p = p + '.ts';
  }

  return p;
}
