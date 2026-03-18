/**
 * Layer Classifier — Pure functions for hexagonal architecture layer classification.
 *
 * Encodes the allowed dependency direction rules as a simple lookup,
 * keeping rule logic testable independently of the analyzer.
 */

import type { DependencyDirection } from '../domain/value-objects.js';

const LAYER_PATTERNS: Array<[string, DependencyDirection]> = [
  // Go conventional directories (more specific first)
  ['/internal/domain/', 'domain'],
  ['/internal/ports/', 'ports'],
  ['/internal/usecases/', 'usecases'],
  ['/internal/', 'usecases'],           // Go: internal/ catch-all → private business logic
  ['/cmd/', 'adapters/primary'],        // Go: cmd/ is the CLI/HTTP entry point
  ['/pkg/', 'ports'],                   // Go: pkg/ is the public API

  // Rust conventional directories
  ['/src/bin/', 'adapters/primary'],    // Rust: src/bin/ contains binary entry points
  ['/src/routes/', 'adapters/primary'], // Rust: web route handlers (actix/axum convention)
  ['/src/handlers/', 'adapters/primary'], // Rust/Go: HTTP handler modules
  ['/src/middleware/', 'adapters/primary'], // Rust/Go: HTTP middleware

  // Go naming conventions (suffix-based)
  ['/handlers/', 'adapters/primary'],   // Go: handler packages

  // Hex-standard patterns (generic, checked last)
  ['/domain/', 'domain'],
  ['/ports/', 'ports'],
  ['/usecases/', 'usecases'],
  ['/adapters/primary/', 'adapters/primary'],
  ['/adapters/secondary/', 'adapters/secondary'],
  ['/infrastructure/', 'infrastructure'],
];

/** Filename-based patterns for special files (checked after directory patterns) */
const FILENAME_PATTERNS: Array<[RegExp, DependencyDirection | 'composition-root' | 'entry-point']> = [
  // Rust special files
  [/\/lib\.rs$/, 'composition-root'],
  [/\/main\.rs$/, 'entry-point'],
  [/\/embed\.rs$/, 'infrastructure'],
  [/\/daemon\.rs$/, 'infrastructure'],

  // Go special files
  [/\/composition-root\.go$/, 'composition-root'],
  [/_adapter\.go$/, 'adapters/primary'],          // *_adapter.go convention
  [/_service\.go$/, 'usecases'],                  // *_service.go convention
  [/\/handler_[^/]+\.go$/, 'adapters/primary'],  // handler_*.go convention
];

/** Allowed import targets for each layer */
const ALLOWED_IMPORTS: Record<DependencyDirection, ReadonlySet<DependencyDirection>> = {
  'domain':             new Set<DependencyDirection>([]),
  'ports':              new Set<DependencyDirection>(['domain']),
  'usecases':           new Set<DependencyDirection>(['domain', 'ports']),
  'adapters/primary':   new Set<DependencyDirection>(['ports']),
  'adapters/secondary': new Set<DependencyDirection>(['ports']),
  'infrastructure':     new Set<DependencyDirection>(['ports']),
};

const VIOLATION_RULES: Record<string, string> = {
  'domain->ports':              'domain must not import from ports (use domain/value-objects)',
  'domain->usecases':           'domain must not import from usecases',
  'domain->adapters/primary':   'domain must not import from adapters',
  'domain->adapters/secondary': 'domain must not import from adapters',
  'domain->infrastructure':     'domain must not import from infrastructure',
  'ports->usecases':            'ports must not import from usecases',
  'ports->adapters/primary':    'ports must not import from adapters',
  'ports->adapters/secondary':  'ports must not import from adapters',
  'ports->infrastructure':      'ports must not import from infrastructure',
  'usecases->adapters/primary':   'usecases may only import from domain and ports',
  'usecases->adapters/secondary': 'usecases may only import from domain and ports',
  'usecases->infrastructure':     'usecases may only import from domain and ports',
  'adapters/primary->domain':             'adapters must not import from domain directly',
  'adapters/primary->usecases':           'adapters must not import from usecases',
  'adapters/primary->adapters/secondary': 'adapters must not import from other adapters',
  'adapters/primary->infrastructure':     'adapters must not import from infrastructure',
  'adapters/secondary->domain':           'adapters must not import from domain directly',
  'adapters/secondary->usecases':         'adapters must not import from usecases',
  'adapters/secondary->adapters/primary': 'adapters must not import from other adapters',
  'adapters/secondary->infrastructure':   'adapters must not import from infrastructure',
  'infrastructure->domain':             'infrastructure may import from ports only',
  'infrastructure->usecases':           'infrastructure may import from ports only',
  'infrastructure->adapters/primary':   'infrastructure may import from ports only',
  'infrastructure->adapters/secondary': 'infrastructure may import from ports only',
};

export function classifyLayer(filePath: string): DependencyDirection | 'unknown' {
  // Prefix with / so patterns like /cmd/ match paths starting with cmd/
  const normalized = '/' + filePath;

  // Skip Go test files — they mirror the package they test, not a distinct layer
  if (normalized.endsWith('_test.go')) return 'unknown';

  // Check directory-based patterns first
  for (const [pattern, layer] of LAYER_PATTERNS) {
    if (normalized.includes(pattern)) return layer;
  }

  // Check filename-based patterns (special files like lib.rs, main.rs, *_adapter.go)
  for (const [regex, classification] of FILENAME_PATTERNS) {
    if (regex.test(normalized)) {
      // composition-root and entry-point are recognized but not hex layers
      if (classification === 'composition-root' || classification === 'entry-point') {
        return 'unknown';
      }
      return classification;
    }
  }

  return 'unknown';
}

export function isAllowedImport(
  fromLayer: DependencyDirection,
  toLayer: DependencyDirection,
): boolean {
  if (fromLayer === toLayer) return true;
  return ALLOWED_IMPORTS[fromLayer]?.has(toLayer) ?? false;
}

/**
 * Classify special files that don't fit neatly into hex layers.
 * Returns a role label or null if not a recognised special file.
 */
export function classifySpecialFile(filePath: string): string | null {
  const normalized = filePath.replace(/\\/g, '/');
  const basename = normalized.split('/').pop() ?? '';

  // Composition roots
  if (basename === 'lib.rs' || basename.startsWith('composition-root')) return 'composition-root';
  // Entry points
  if (basename === 'main.rs' || basename === 'main.go' || basename === 'main.ts') return 'entry-point';
  // Build files
  if (basename === 'build.rs' || basename === 'Cargo.toml') return 'build-config';

  return null;
}

export function getViolationRule(
  fromLayer: DependencyDirection,
  toLayer: DependencyDirection,
): string | null {
  if (isAllowedImport(fromLayer, toLayer)) return null;
  return VIOLATION_RULES[`${fromLayer}->${toLayer}`] ?? `${fromLayer} must not import from ${toLayer}`;
}
