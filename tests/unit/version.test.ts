import { describe, expect, it } from 'bun:test';
import { Version } from '../../src/core/domain/value-objects.js';

describe('Version (CalVer YY.M)', () => {
  describe('parse', () => {
    it('parses YY.M format', () => {
      const v = Version.parse('26.3');
      expect(v).not.toBeNull();
      expect(v!.year).toBe(26);
      expect(v!.month).toBe(3);
      expect(v!.patch).toBe(0);
    });

    it('parses YY.M.patch format', () => {
      const v = Version.parse('26.3.1');
      expect(v).not.toBeNull();
      expect(v!.year).toBe(26);
      expect(v!.month).toBe(3);
      expect(v!.patch).toBe(1);
    });

    it('returns null for invalid strings', () => {
      expect(Version.parse('abc')).toBeNull();
      expect(Version.parse('')).toBeNull();
      expect(Version.parse('1')).toBeNull();
      expect(Version.parse('1.2.3.4')).toBeNull();
    });

    it('returns null for month > 12', () => {
      expect(Version.parse('26.13')).toBeNull();
    });

    it('returns null for month < 1', () => {
      expect(Version.parse('26.0')).toBeNull();
    });

    it('returns null for negative patch', () => {
      expect(Version.parse('26.3.-1')).toBeNull();
    });
  });

  describe('comparison', () => {
    it('newer year wins', () => {
      const v1 = Version.parse('27.1')!;
      const v2 = Version.parse('26.12')!;
      expect(v1.isNewerThan(v2)).toBe(true);
      expect(v2.isNewerThan(v1)).toBe(false);
    });

    it('newer month wins within same year', () => {
      const v1 = Version.parse('26.3')!;
      const v2 = Version.parse('26.2.9')!;
      expect(v1.isNewerThan(v2)).toBe(true);
    });

    it('patch breaks ties', () => {
      const v1 = Version.parse('26.3.1')!;
      const v2 = Version.parse('26.3.0')!;
      expect(v1.isNewerThan(v2)).toBe(true);
      expect(v2.isNewerThan(v1)).toBe(false);
    });

    it('equal versions are equal', () => {
      const v1 = Version.parse('26.3')!;
      const v2 = Version.parse('26.3.0')!;
      expect(v1.equals(v2)).toBe(true);
      expect(v1.isNewerThan(v2)).toBe(false);
    });
  });

  describe('toString', () => {
    it('omits patch when zero', () => {
      expect(Version.parse('26.3')!.toString()).toBe('26.3');
    });

    it('includes patch when non-zero', () => {
      expect(Version.parse('26.3.1')!.toString()).toBe('26.3.1');
    });
  });
});
