/**
 * Property Tests — Layer Classifier
 *
 * These tests verify universal invariants over ALL possible inputs,
 * not just specific examples. They catch rule-table gaps that unit
 * tests (which only check known-good paths) would miss.
 */

import { describe, it, expect } from 'bun:test';
import {
  classifyLayer,
  isAllowedImport,
  getViolationRule,
} from '../../src/core/usecases/layer-classifier.js';
import type { DependencyDirection } from '../../src/core/domain/value-objects.js';

// ── Constants ───────────────────────────────────────────────

const ALL_LAYERS: DependencyDirection[] = [
  'domain',
  'ports',
  'usecases',
  'adapters/primary',
  'adapters/secondary',
  'infrastructure',
];

/** Paths that should map to each layer, across all supported languages */
const LAYER_PATHS: Record<DependencyDirection, string[]> = {
  domain: [
    'src/core/domain/entities.ts',
    'src/core/domain/value-objects.ts',
    'internal/domain/model.go',
  ],
  ports: [
    'src/core/ports/index.ts',
    'src/core/ports/swarm.ts',
    'internal/ports/storage.go',
    'pkg/api.go',
  ],
  usecases: [
    'src/core/usecases/arch-analyzer.ts',
    'src/core/usecases/summary-service.ts',
    'internal/usecases/handler.go',
  ],
  'adapters/primary': [
    'src/adapters/primary/cli-adapter.ts',
    'src/adapters/primary/mcp-adapter.ts',
    'cmd/server/main.go',
    'src/bin/main.rs',
    'src/routes/api.rs',
    'src/handlers/auth.rs',
  ],
  'adapters/secondary': [
    'src/adapters/secondary/filesystem-adapter.ts',
    'src/adapters/secondary/git-adapter.ts',
  ],
  infrastructure: [
    'src/infrastructure/treesitter/queries.ts',
  ],
};

// ── Property: Classification completeness ───────────────────

describe('Property: classifyLayer completeness', () => {
  for (const layer of ALL_LAYERS) {
    for (const path of LAYER_PATHS[layer]) {
      it(`classifies "${path}" as "${layer}"`, () => {
        expect(classifyLayer(path)).toBe(layer);
      });
    }
  }
});

// ── Property: Reflexivity — same-layer imports always allowed ──

describe('Property: same-layer imports are always allowed', () => {
  for (const layer of ALL_LAYERS) {
    it(`${layer} → ${layer} is allowed`, () => {
      expect(isAllowedImport(layer, layer)).toBe(true);
    });
  }
});

// ── Property: Every disallowed pair has a violation rule ─────

describe('Property: every disallowed import has a violation rule', () => {
  for (const from of ALL_LAYERS) {
    for (const to of ALL_LAYERS) {
      if (from === to) continue;
      if (isAllowedImport(from, to)) continue;

      it(`${from} → ${to} has a non-empty violation rule`, () => {
        const rule = getViolationRule(from, to);
        expect(rule).toBeTypeOf('string');
        expect(rule!.length).toBeGreaterThan(0);
      });
    }
  }
});

// ── Property: Every allowed pair returns null violation ──────

describe('Property: allowed imports have no violation rule', () => {
  for (const from of ALL_LAYERS) {
    for (const to of ALL_LAYERS) {
      if (!isAllowedImport(from, to)) continue;

      it(`${from} → ${to} returns null violation`, () => {
        expect(getViolationRule(from, to)).toBeNull();
      });
    }
  }
});

// ── Property: Domain is a sink (no outward deps) ────────────

describe('Property: domain is a dependency sink', () => {
  for (const target of ALL_LAYERS) {
    if (target === 'domain') continue;

    it(`domain → ${target} is forbidden`, () => {
      expect(isAllowedImport('domain', target)).toBe(false);
    });
  }
});

// ── Property: Adapters never import other adapters ──────────

describe('Property: cross-adapter imports are forbidden', () => {
  const adapterLayers: DependencyDirection[] = ['adapters/primary', 'adapters/secondary'];

  for (const from of adapterLayers) {
    for (const to of adapterLayers) {
      if (from === to) continue;

      it(`${from} → ${to} is forbidden`, () => {
        expect(isAllowedImport(from, to)).toBe(false);
      });
    }
  }
});

// ── Property: isAllowedImport ↔ getViolationRule consistency ─

describe('Property: isAllowedImport and getViolationRule are consistent', () => {
  for (const from of ALL_LAYERS) {
    for (const to of ALL_LAYERS) {
      it(`${from} → ${to}: allowed=${isAllowedImport(from, to)} matches rule=${getViolationRule(from, to) === null}`, () => {
        const allowed = isAllowedImport(from, to);
        const rule = getViolationRule(from, to);

        if (allowed) {
          expect(rule).toBeNull();
        } else {
          expect(rule).not.toBeNull();
        }
      });
    }
  }
});
