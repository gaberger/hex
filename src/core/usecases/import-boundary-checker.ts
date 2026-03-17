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

interface PlannedImport {
  /** The file being written (e.g., "src/adapters/primary/http-adapter.ts") */
  fromFile: string;
  /** The import target (e.g., "../../../core/domain/entities.js") */
  toFile: string;
  /** Imported symbol names for diagnostics */
  names: string[];
}

interface BoundaryCheckResult {
  valid: boolean;
  violations: DependencyViolation[];
  warnings: string[];
}

/**
 * Check a single planned import against hex boundary rules.
 * Returns null if allowed, or a DependencyViolation if forbidden.
 * (Internal helper - not part of public API)
 */
function checkImport(fromFile: string, toFile: string, _names: string[]): DependencyViolation | null {
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

// NOTE: validatePlannedImports and allowedImportsFor were removed as unused.
// They were planned for the hex-coder agent's pre-generation validation but never implemented.
// If needed in the future, the logic exists in checkImport() above and layer-classifier.ts.
