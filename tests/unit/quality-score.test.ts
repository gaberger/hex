import { describe, it, expect } from 'bun:test';
import { QualityScore } from '../../src/core/domain/entities.js';
import { makeQualityScore } from './fixtures.js';

describe('QualityScore', () => {
  describe('score', () => {
    it('returns 0 when compileSuccess is false', () => {
      const qs = makeQualityScore({ compileSuccess: false });
      expect(qs.score).toBe(0);
    });

    it('returns high value when all gates pass', () => {
      const qs = makeQualityScore({
        compileSuccess: true,
        lintErrorCount: 0,
        lintWarningCount: 0,
        testsPassed: 10,
        testsFailed: 0,
        tokenEfficiency: 0.2,
      });
      expect(qs.score).toBe(100);
    });

    it('penalizes lint errors at 10 points each', () => {
      const clean = makeQualityScore({ lintErrorCount: 0 });
      const oneError = makeQualityScore({ lintErrorCount: 1 });
      expect(clean.score - oneError.score).toBe(10);
    });

    it('penalizes lint warnings at 2 points each', () => {
      const clean = makeQualityScore({ lintWarningCount: 0 });
      const oneWarning = makeQualityScore({ lintWarningCount: 1 });
      expect(clean.score - oneWarning.score).toBe(2);
    });

    it('weights test pass ratio at 60%', () => {
      const allPass = makeQualityScore({ testsPassed: 10, testsFailed: 0, tokenEfficiency: 0 });
      const halfPass = makeQualityScore({ testsPassed: 5, testsFailed: 5, tokenEfficiency: 0 });
      // allPass testScore=1*60=60, halfPass testScore=0.5*60=30; difference=30
      expect(allPass.score - halfPass.score).toBe(30);
    });

    it('weights token efficiency at 20%', () => {
      const efficient = makeQualityScore({
        testsPassed: 0,
        testsFailed: 0,
        lintErrorCount: 0,
        lintWarningCount: 0,
        tokenEfficiency: 0.2,
      });
      const inefficient = makeQualityScore({
        testsPassed: 0,
        testsFailed: 0,
        lintErrorCount: 0,
        lintWarningCount: 0,
        tokenEfficiency: 0,
      });
      // efficient: testScore=0/1*60=0, eff=min(1,1)*20=20, lint=20 => 40
      // inefficient: testScore=0, eff=0, lint=20 => 20
      expect(efficient.score - inefficient.score).toBe(20);
    });

    it('is clamped between 0 and 100', () => {
      const worst = makeQualityScore({
        compileSuccess: true,
        lintErrorCount: 50,
        testsPassed: 0,
        testsFailed: 10,
        tokenEfficiency: 0,
      });
      expect(worst.score).toBeGreaterThanOrEqual(0);

      const best = makeQualityScore();
      expect(best.score).toBeLessThanOrEqual(100);
    });
  });

  describe('passing', () => {
    it('returns true only when compile succeeds, zero lint errors, zero test failures', () => {
      const qs = makeQualityScore({
        compileSuccess: true,
        lintErrorCount: 0,
        testsFailed: 0,
      });
      expect(qs.passing).toBe(true);
    });

    it('returns false with any lint errors', () => {
      const qs = makeQualityScore({ lintErrorCount: 1 });
      expect(qs.passing).toBe(false);
    });

    it('returns false when compile fails', () => {
      const qs = makeQualityScore({ compileSuccess: false });
      expect(qs.passing).toBe(false);
    });

    it('returns false with test failures', () => {
      const qs = makeQualityScore({ testsFailed: 1 });
      expect(qs.passing).toBe(false);
    });
  });
});
