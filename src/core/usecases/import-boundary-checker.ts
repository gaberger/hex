/**
 * Import Boundary Checker — Pre-generation validation
 *
 * Intervention A: Validates planned imports BEFORE code is written.
 * Called by hex-coder agent before each file write to ensure no hex
 * boundary violations can be introduced. Uses the same layer-classifier
 * rules as the post-build arch-analyzer, but operates on planned
 * import paths rather than existing AST summaries.
 *
 * This is the "shift-left" companion to arch-analyzer.ts — same rules,
 * earlier enforcement.
 */

import { classifyLayer, isAllowedImport, getViolationRule } from './layer-classifier.js';
import type { DependencyDirection, DependencyViolation } from '../domain/value-objects.js';

export interface PlannedImport {
  /** The file being written (e.g., "src/adapters/primary/http-adapter.ts") */
  fromFile: string;
  /** The import target (e.g., "../../../core/domain/entities.js") */
  toFile: string;
  /** Imported symbol names for diagnostics */
  names: string[];
}

export interface BoundaryCheckResult {
  valid: boolean;
  violations: DependencyViolation[];
  warnings: string[];
}

/**
 * Check a single planned import against hex boundary rules.
 * Returns null if allowed, or a DependencyViolation if forbidden.
 */
export function checkImport(fromFile: string, toFile: string, names: string[]): DependencyViolation | null {
  const fromLayer = classifyLayer(fromFile);
  const toLayer = classifyLayer(toFile);

  // Unknown layers can't be validated — warn but don't block
  if (fromLayer === 'unknown' || toLayer === 'unknown') return null;

  // Same-layer imports are always allowed
  if (fromLayer === toLayer) return null;

  if (isAllowedImport(fromLayer as DependencyDirection, toLayer as DependencyDirection)) {
    return null;
  }

  const rule = getViolationRule(fromLayer as DependencyDirection, toLayer as DependencyDirection);
  return {
    from: fromFile,
    to: toFile,
    fromLayer: fromLayer as DependencyDirection,
    toLayer: toLayer as DependencyDirection,
    rule: rule ?? `${fromLayer} must not import from ${toLayer}`,
  };
}

/**
 * Validate all planned imports for a file before writing it.
 * Returns a BoundaryCheckResult with any violations found.
 *
 * Usage in hex-coder agent (before writing each file):
 *   const result = validatePlannedImports(filePath, plannedImports);
 *   if (!result.valid) {
 *     // DO NOT write this file — fix imports first
 *     console.error('Boundary violations:', result.violations);
 *   }
 */
export function validatePlannedImports(
  filePath: string,
  imports: PlannedImport[],
): BoundaryCheckResult {
  const violations: DependencyViolation[] = [];
  const warnings: string[] = [];

  for (const imp of imports) {
    const violation = checkImport(imp.fromFile, imp.toFile, imp.names);
    if (violation) {
      violations.push(violation);
    }
  }

  // Warn about patterns that are technically allowed but smell bad
  const fromLayer = classifyLayer(filePath);
  if (fromLayer !== 'unknown') {
    const domainImports = imports.filter((i) => classifyLayer(i.toFile) === 'domain');
    if ((fromLayer === 'adapters/primary' || fromLayer === 'adapters/secondary') && domainImports.length > 0) {
      // This IS a violation per hex rules — adapters should go through ports
      // But if layer-classifier allows it, at minimum warn
      warnings.push(
        `Adapter "${filePath}" imports ${domainImports.length} domain module(s) directly. ` +
        `Consider re-exporting needed types through ports instead.`
      );
    }
  }

  return {
    valid: violations.length === 0,
    violations,
    warnings,
  };
}

/**
 * Quick check: given a source layer, return what layers it MAY import from.
 * Useful for hex-coder to know upfront what's allowed.
 */
export function allowedImportsFor(filePath: string): DependencyDirection[] {
  const layer = classifyLayer(filePath);
  if (layer === 'unknown') return [];

  const allowed: DependencyDirection[] = [layer as DependencyDirection]; // same-layer always OK
  const allLayers: DependencyDirection[] = [
    'domain', 'ports', 'usecases',
    'adapters/primary', 'adapters/secondary', 'infrastructure',
  ];

  for (const target of allLayers) {
    if (target !== layer && isAllowedImport(layer as DependencyDirection, target)) {
      allowed.push(target);
    }
  }

  return allowed;
}
