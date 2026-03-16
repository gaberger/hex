import { describe, it, expect } from 'bun:test';
import { SerializationAdapter } from '../../src/adapters/secondary/serialization-adapter.js';
import type { SerializedPayload } from '../../src/core/ports/cross-lang.js';

describe('SerializationAdapter', () => {
  const adapter = () => new SerializationAdapter();

  // ── supportsFormat ────────────────────────────────────────

  describe('supportsFormat', () => {
    it('returns true for json', () => {
      expect(adapter().supportsFormat('json')).toBe(true);
    });

    it('returns false for protobuf', () => {
      expect(adapter().supportsFormat('protobuf')).toBe(false);
    });

    it('returns false for messagepack', () => {
      expect(adapter().supportsFormat('messagepack')).toBe(false);
    });
  });

  // ── JSON roundtrip ────────────────────────────────────────

  describe('serialize / deserialize roundtrip', () => {
    it('roundtrips a plain object', async () => {
      const a = adapter();
      const original = { name: 'hex', version: 3 };
      const payload = await a.serialize(original, 'json', 'Config');
      const restored = await a.deserialize<typeof original>(payload);
      expect(restored).toEqual(original);
    });

    it('roundtrips an array', async () => {
      const a = adapter();
      const original = [1, 'two', null, true];
      const payload = await a.serialize(original, 'json', 'MixedArray');
      const restored = await a.deserialize<typeof original>(payload);
      expect(restored).toEqual(original);
    });

    it('roundtrips primitives', async () => {
      const a = adapter();

      const numPayload = await a.serialize(42, 'json', 'Num');
      expect(await a.deserialize<number>(numPayload)).toBe(42);

      const strPayload = await a.serialize('hello', 'json', 'Str');
      expect(await a.deserialize<string>(strPayload)).toBe('hello');

      const boolPayload = await a.serialize(true, 'json', 'Bool');
      expect(await a.deserialize<boolean>(boolPayload)).toBe(true);

      const nullPayload = await a.serialize(null, 'json', 'Null');
      expect(await a.deserialize<null>(nullPayload)).toBe(null);
    });

    it('produces a SerializedPayload with correct metadata', async () => {
      const a = adapter();
      const payload = await a.serialize({ a: 1 }, 'json', 'Obj');
      expect(payload.format).toBe('json');
      expect(payload.typeName).toBe('Obj');
      expect(payload.data).toBeInstanceOf(Uint8Array);
    });
  });

  // ── Unsupported format errors ─────────────────────────────

  describe('unsupported formats', () => {
    it('serialize throws for protobuf', async () => {
      await expect(adapter().serialize({}, 'protobuf', 'X')).rejects.toThrow(
        'Format not supported: protobuf',
      );
    });

    it('serialize throws for messagepack', async () => {
      await expect(adapter().serialize({}, 'messagepack', 'X')).rejects.toThrow(
        'Format not supported: messagepack',
      );
    });

    it('deserialize throws for unsupported format', async () => {
      const payload: SerializedPayload = {
        format: 'protobuf',
        data: new Uint8Array(),
        typeName: 'X',
      };
      await expect(adapter().deserialize(payload)).rejects.toThrow(
        'Format not supported: protobuf',
      );
    });
  });

  // ── registeredTypes ───────────────────────────────────────

  describe('registeredTypes', () => {
    it('returns empty array for format with no types', () => {
      expect(adapter().registeredTypes('json')).toEqual([]);
    });

    it('tracks types registered via registerType', () => {
      const a = adapter();
      a.registerType('User', 'json');
      a.registerType('Config', 'json');
      const types = a.registeredTypes('json');
      expect(types).toContain('User');
      expect(types).toContain('Config');
      expect(types).toHaveLength(2);
    });

    it('tracks types registered implicitly via serialize', async () => {
      const a = adapter();
      await a.serialize({ id: 1 }, 'json', 'Item');
      expect(a.registeredTypes('json')).toContain('Item');
    });

    it('does not duplicate type names', async () => {
      const a = adapter();
      a.registerType('Dup', 'json');
      a.registerType('Dup', 'json');
      await a.serialize({}, 'json', 'Dup');
      expect(a.registeredTypes('json')).toHaveLength(1);
    });

    it('isolates types by format', () => {
      const a = adapter();
      a.registerType('Foo', 'json');
      a.registerType('Bar', 'protobuf');
      expect(a.registeredTypes('json')).toEqual(['Foo']);
      expect(a.registeredTypes('protobuf')).toEqual(['Bar']);
    });
  });
});
