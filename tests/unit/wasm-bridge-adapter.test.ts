import { describe, it, expect, beforeEach } from 'bun:test';
import {
  WASMBridgeAdapter,
  WASMModuleNotFoundError,
  WASMFunctionNotFoundError,
} from '../../src/adapters/secondary/wasm-bridge-adapter.js';
import type {
  WASMModuleDescriptor,
  SerializedPayload,
} from '../../src/core/ports/cross-lang.js';

function makeDescriptor(overrides?: Partial<WASMModuleDescriptor>): WASMModuleDescriptor {
  return {
    source: '/tmp/test-module.wasm',
    sourceLanguage: 'rust',
    exports: ['add', 'multiply'],
    memory: { initial: 1 },
    ...overrides,
  };
}

function jsonPayload(value: unknown): SerializedPayload {
  return {
    format: 'json',
    data: new TextEncoder().encode(JSON.stringify(value)),
    typeName: typeof value,
  };
}

describe('WASMBridgeAdapter', () => {
  let adapter: WASMBridgeAdapter;

  beforeEach(() => {
    adapter = new WASMBridgeAdapter();
  });

  describe('listModules', () => {
    it('should return empty array initially', () => {
      expect(adapter.listModules()).toEqual([]);
    });
  });

  describe('isLoaded', () => {
    it('should return false for modules that were never loaded', () => {
      expect(adapter.isLoaded('nonexistent')).toBe(false);
    });
  });

  describe('call', () => {
    it('should throw WASMModuleNotFoundError for unknown module', async () => {
      await expect(
        adapter.call('unknown', 'fn', []),
      ).rejects.toBeInstanceOf(WASMModuleNotFoundError);
    });

    it('should include module name in error message', async () => {
      await expect(
        adapter.call('my-module', 'fn', []),
      ).rejects.toThrow('WASM module not loaded: "my-module"');
    });
  });

  describe('unload', () => {
    it('should throw WASMModuleNotFoundError for unknown module', async () => {
      await expect(
        adapter.unload('missing'),
      ).rejects.toBeInstanceOf(WASMModuleNotFoundError);
    });
  });

  describe('load', () => {
    it('should throw when .wasm file does not exist', async () => {
      const descriptor = makeDescriptor({ source: '/nonexistent/path.wasm' });
      await expect(adapter.load(descriptor)).rejects.toThrow();
    });
  });

  describe('WASMFunctionNotFoundError', () => {
    it('should contain module and function names in message', () => {
      const err = new WASMFunctionNotFoundError('mod', 'func');
      expect(err.message).toBe('Function "func" not found in WASM module "mod"');
      expect(err.name).toBe('WASMFunctionNotFoundError');
    });
  });

  describe('jsonPayload helper', () => {
    it('should produce valid JSON serialized payload', () => {
      const p = jsonPayload(42);
      expect(p.format).toBe('json');
      const decoded = JSON.parse(new TextDecoder().decode(p.data));
      expect(decoded).toBe(42);
    });
  });
});
