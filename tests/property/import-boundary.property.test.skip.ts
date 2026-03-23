/**
 * Property Tests — Import Boundary Checker
 *
 * Verifies that the pre-generation boundary checker is consistent
 * with the layer classifier's rules. This is a critical cross-module
 * property: if checkImport disagrees with isAllowedImport, the
 * "shift-left" guarantee is broken.
 */

import { describe, it, expect } from 'bun:test';
import {
  checkImport,
  validatePlannedImports,
  allowedImportsFor,
} from '../../src/core/usecases/import-boundary-checker.js';
import {
  classifyLayer,
  isAllowedImport,
} from '../../src/core/usecases/layer-classifier.js';
import type { DependencyDirection } from '../../src/core/domain/value-objects.js';

// ── Helpers ─────────────────────────────────────────────────

/** Representative file for each layer */
const LAYER_FILES: Record<DependencyDirection, string> = {
  domain: 'src/core/domain/entities.ts',
  ports: 'src/core/ports/index.ts',
  usecases: 'src/core/usecases/arch-analyzer.ts',
  'adapters/primary': 'src/adapters/primary/cli-adapter.ts',
  'adapters/secondary': 'src/adapters/secondary/filesystem-adapter.ts',
  infrastructure: 'src/infrastructure/treesitter/queries.ts',
};

const ALL_LAYERS: DependencyDirection[] = Object.keys(LAYER_FILES) as DependencyDirection[];

// ── Property: checkImport agrees with isAllowedImport ───────

describe('Property: checkImport is consistent with isAllowedImport', () => {
  for (const from of ALL_LAYERS) {
    for (const to of ALL_LAYERS) {
      const fromFile = LAYER_FILES[from];
      const toFile = LAYER_FILES[to];

      it(`checkImport(${from}, ${to}) agrees with isAllowedImport`, () => {
        const violation = checkImport(fromFile, toFile, ['SomeSymbol']);
        const allowed = isAllowedImport(from, to);

        if (allowed) {
          expect(violation).toBeNull();
        } else {
          expect(violation).not.toBeNull();
          expect(violation!.fromLayer).toBe(from);
          expect(violation!.toLayer).toBe(to);
          expect(violation!.rule.length).toBeGreaterThan(0);
        }
      });
    }
  }
});

// ── Property: validatePlannedImports returns valid=true iff no violations ──

describe('Property: validatePlannedImports valid flag matches violation count', () => {
  for (const from of ALL_LAYERS) {
    const fromFile = LAYER_FILES[from];

    // Build a mix of allowed and disallowed imports
    const imports = ALL_LAYERS.map((to) => ({
      fromFile,
      toFile: LAYER_FILES[to],
      names: ['Foo'],
    }));

    it(`validatePlannedImports for ${from} has correct valid flag`, () => {
      const result = validatePlannedImports(fromFile, imports);

      if (result.violations.length === 0) {
        expect(result.valid).toBe(true);
      } else {
        expect(result.valid).toBe(false);
      }
    });
  }
});

// ── Property: allowedImportsFor always includes own layer ───

describe('Property: allowedImportsFor always includes own layer', () => {
  for (const layer of ALL_LAYERS) {
    const file = LAYER_FILES[layer];

    it(`allowedImportsFor("${file}") includes "${layer}"`, () => {
      const allowed = allowedImportsFor(file);
      expect(allowed).toContain(layer);
    });
  }
});

// ── Property: allowedImportsFor matches isAllowedImport ─────

describe('Property: allowedImportsFor is consistent with isAllowedImport', () => {
  for (const from of ALL_LAYERS) {
    const fromFile = LAYER_FILES[from];
    const allowed = allowedImportsFor(fromFile);

    for (const to of ALL_LAYERS) {
      it(`${from} → ${to}: allowedImportsFor includes=${allowed.includes(to)}, isAllowed=${isAllowedImport(from, to)}`, () => {
        expect(allowed.includes(to)).toBe(isAllowedImport(from, to));
      });
    }
  }
});

// ── Property: Unknown files produce no violations ───────────

describe('Property: unknown-layer files never produce violations', () => {
  const unknownFiles = [
    'README.md',
    'package.json',
    'tsconfig.json',
    '.gitignore',
  ];

  for (const unknownFile of unknownFiles) {
    for (const layer of ALL_LAYERS) {
      it(`checkImport("${unknownFile}", "${LAYER_FILES[layer]}") returns null`, () => {
        expect(checkImport(unknownFile, LAYER_FILES[layer], ['X'])).toBeNull();
      });

      it(`checkImport("${LAYER_FILES[layer]}", "${unknownFile}") returns null`, () => {
        expect(checkImport(LAYER_FILES[layer], unknownFile, ['X'])).toBeNull();
      });
    }
  }
});
