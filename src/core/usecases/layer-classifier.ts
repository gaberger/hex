/**
 * Layer Classifier — Pure functions for hexagonal architecture layer classification.
 *
 * Encodes the allowed dependency direction rules as a simple lookup,
 * keeping rule logic testable independently of the analyzer.
 */

import type { DependencyDirection } from '../ports/index.js';

const LAYER_PATTERNS: Array<[string, DependencyDirection]> = [
  ['/domain/', 'domain'],
  ['/ports/', 'ports'],
  ['/usecases/', 'usecases'],
  ['/adapters/primary/', 'adapters/primary'],
  ['/adapters/secondary/', 'adapters/secondary'],
  ['/infrastructure/', 'infrastructure'],
];

/** Allowed import targets for each layer */
const ALLOWED_IMPORTS: Record<DependencyDirection, ReadonlySet<DependencyDirection>> = {
  'domain':             new Set<DependencyDirection>(['ports']),
  'ports':              new Set<DependencyDirection>(['domain']),
  'usecases':           new Set<DependencyDirection>(['domain', 'ports']),
  'adapters/primary':   new Set<DependencyDirection>(['ports']),
  'adapters/secondary': new Set<DependencyDirection>(['ports']),
  'infrastructure':     new Set<DependencyDirection>(['ports']),
};

const VIOLATION_RULES: Record<string, string> = {
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
  for (const [pattern, layer] of LAYER_PATTERNS) {
    if (filePath.includes(pattern)) return layer;
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

export function getViolationRule(
  fromLayer: DependencyDirection,
  toLayer: DependencyDirection,
): string | null {
  if (isAllowedImport(fromLayer, toLayer)) return null;
  return VIOLATION_RULES[`${fromLayer}->${toLayer}`] ?? `${fromLayer} must not import from ${toLayer}`;
}
