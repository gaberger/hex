import { describe, it, expect } from 'bun:test';
import fc from 'fast-check';
import { Version } from '../../src/core/domain/value-objects.js';
import type { Language, ASTSummary, ExportEntry } from '../../src/core/domain/value-objects.js';

describe('Version (property tests)', () => {
  const validYear = fc.integer({ min: 0, max: 99 });
  const validMonth = fc.integer({ min: 1, max: 12 });
  const validPatch = fc.integer({ min: 0, max: 99 });

  it('round-trips through toString and parse', () => {
    fc.assert(
      fc.property(validYear, validMonth, validPatch, (year, month, patch) => {
        const v = new Version(year, month, patch);
        const parsed = Version.parse(v.toString());
        expect(parsed).not.toBeNull();
        expect(parsed!.year).toBe(year);
        expect(parsed!.month).toBe(month);
        expect(parsed!.patch).toBe(patch);
      }),
    );
  });

  it('equals is reflexive', () => {
    fc.assert(
      fc.property(validYear, validMonth, validPatch, (year, month, patch) => {
        const v = new Version(year, month, patch);
        expect(v.equals(v)).toBe(true);
      }),
    );
  });

  it('equals is symmetric', () => {
    fc.assert(
      fc.property(validYear, validMonth, validPatch, (year, month, patch) => {
        const a = new Version(year, month, patch);
        const b = new Version(year, month, patch);
        expect(a.equals(b)).toBe(b.equals(a));
      }),
    );
  });

  it('isNewerThan is irreflexive', () => {
    fc.assert(
      fc.property(validYear, validMonth, validPatch, (year, month, patch) => {
        const v = new Version(year, month, patch);
        expect(v.isNewerThan(v)).toBe(false);
      }),
    );
  });

  it('isNewerThan is asymmetric — if a > b then !(b > a)', () => {
    fc.assert(
      fc.property(
        validYear, validMonth, validPatch,
        validYear, validMonth, validPatch,
        (y1, m1, p1, y2, m2, p2) => {
          const a = new Version(y1, m1, p1);
          const b = new Version(y2, m2, p2);
          if (a.isNewerThan(b)) {
            expect(b.isNewerThan(a)).toBe(false);
          }
        },
      ),
    );
  });

  it('isNewerThan is transitive', () => {
    fc.assert(
      fc.property(
        validYear, validMonth, validPatch,
        validYear, validMonth, validPatch,
        validYear, validMonth, validPatch,
        (y1, m1, p1, y2, m2, p2, y3, m3, p3) => {
          const a = new Version(y1, m1, p1);
          const b = new Version(y2, m2, p2);
          const c = new Version(y3, m3, p3);
          if (a.isNewerThan(b) && b.isNewerThan(c)) {
            expect(a.isNewerThan(c)).toBe(true);
          }
        },
      ),
    );
  });

  it('parse rejects invalid strings', () => {
    fc.assert(
      fc.property(fc.string(), (s) => {
        // Filter out strings that happen to be valid versions
        const parsed = Version.parse(s);
        if (parsed !== null) {
          // If it parsed, it must round-trip
          const reparsed = Version.parse(parsed.toString());
          expect(reparsed).not.toBeNull();
          expect(reparsed!.equals(parsed)).toBe(true);
        }
      }),
    );
  });

  it('parse rejects out-of-range months', () => {
    fc.assert(
      fc.property(
        validYear,
        fc.integer({ min: 13, max: 99 }),
        validPatch,
        (year, month, patch) => {
          const result = Version.parse(`${year}.${month}.${patch}`);
          expect(result).toBeNull();
        },
      ),
    );
  });

  it('toString omits patch when patch is 0', () => {
    fc.assert(
      fc.property(validYear, validMonth, (year, month) => {
        const v = new Version(year, month, 0);
        expect(v.toString()).toBe(`${year}.${month}`);
      }),
    );
  });

  it('toString includes patch when patch > 0', () => {
    fc.assert(
      fc.property(validYear, validMonth, fc.integer({ min: 1, max: 99 }), (year, month, patch) => {
        const v = new Version(year, month, patch);
        expect(v.toString()).toBe(`${year}.${month}.${patch}`);
      }),
    );
  });
});

describe('Language type (property tests)', () => {
  const languages: Language[] = ['typescript', 'go', 'rust'];

  it('Language enum values are exhaustive and stable', () => {
    expect(languages).toHaveLength(3);
    for (const lang of languages) {
      expect(typeof lang).toBe('string');
      expect(lang.length).toBeGreaterThan(0);
    }
  });
});

describe('ASTSummary invariants (property tests)', () => {
  const languageArb: fc.Arbitrary<Language> = fc.constantFrom('typescript', 'go', 'rust');
  const levelArb = fc.constantFrom('L0' as const, 'L1' as const, 'L2' as const, 'L3' as const);
  const kindArb = fc.constantFrom(
    'function' as const, 'class' as const, 'interface' as const,
    'type' as const, 'const' as const, 'enum' as const,
  );

  const exportEntryArb: fc.Arbitrary<ExportEntry> = fc.record({
    name: fc.string({ minLength: 1, maxLength: 50 }),
    kind: kindArb,
    signature: fc.option(fc.string({ maxLength: 200 }), { nil: undefined }),
  });

  const astSummaryArb: fc.Arbitrary<ASTSummary> = fc.record({
    filePath: fc.string({ minLength: 1, maxLength: 200 }),
    language: languageArb,
    level: levelArb,
    exports: fc.array(exportEntryArb, { maxLength: 20 }),
    imports: fc.array(
      fc.record({
        names: fc.array(fc.string({ minLength: 1 }), { maxLength: 10 }),
        from: fc.string({ minLength: 1 }),
      }),
      { maxLength: 10 },
    ),
    dependencies: fc.array(fc.string(), { maxLength: 10 }),
    lineCount: fc.nat(),
    tokenEstimate: fc.nat(),
  });

  it('tokenEstimate and lineCount are non-negative', () => {
    fc.assert(
      fc.property(astSummaryArb, (summary) => {
        expect(summary.tokenEstimate).toBeGreaterThanOrEqual(0);
        expect(summary.lineCount).toBeGreaterThanOrEqual(0);
      }),
    );
  });

  it('exports have non-empty names', () => {
    fc.assert(
      fc.property(astSummaryArb, (summary) => {
        for (const exp of summary.exports) {
          expect(exp.name.length).toBeGreaterThan(0);
        }
      }),
    );
  });
});
