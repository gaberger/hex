import { describe, it, expect, beforeEach } from 'bun:test';

import { FFIAdapter } from '../../src/adapters/secondary/ffi-adapter.js';
import type { FFILibraryDescriptor } from '../../src/core/ports/cross-lang.js';

describe('FFIAdapter', () => {
  let adapter: FFIAdapter;

  beforeEach(() => {
    adapter = new FFIAdapter();
  });

  // ── listLibraries ─────────────────────────────────────────

  it('returns empty list initially', () => {
    expect(adapter.listLibraries()).toEqual([]);
  });

  // ── isLoaded ──────────────────────────────────────────────

  it('returns false for unloaded library', () => {
    expect(adapter.isLoaded('/nonexistent/lib.so')).toBe(false);
  });

  // ── load ──────────────────────────────────────────────────

  it('throws when loading a non-existent path', async () => {
    const descriptor: FFILibraryDescriptor = {
      path: '/tmp/__hex_ffi_nonexistent_library__',
      sourceLanguage: 'rust',
      symbols: [],
    };

    await expect(adapter.load(descriptor)).rejects.toThrow(
      'Library not found or not executable',
    );
  });

  it('loads an executable path and marks it as loaded', async () => {
    // /bin/echo is a safe, universally available executable
    const descriptor: FFILibraryDescriptor = {
      path: '/bin/echo',
      sourceLanguage: 'go',
      symbols: [{ name: 'ping', signature: '() -> string', allocates: false }],
    };

    await adapter.load(descriptor);
    expect(adapter.isLoaded('/bin/echo')).toBe(true);
    expect(adapter.listLibraries()).toHaveLength(1);
    expect(adapter.listLibraries()[0].path).toBe('/bin/echo');
  });

  // ── call ──────────────────────────────────────────────────

  it('throws when calling an unknown library', async () => {
    await expect(
      adapter.call('/not/loaded', 'fn', []),
    ).rejects.toThrow('Library not loaded');
  });

  it('throws when calling an unknown symbol', async () => {
    const descriptor: FFILibraryDescriptor = {
      path: '/bin/echo',
      sourceLanguage: 'go',
      symbols: [{ name: 'ping', signature: '() -> string', allocates: false }],
    };

    await adapter.load(descriptor);

    await expect(
      adapter.call('/bin/echo', 'nonexistent', []),
    ).rejects.toThrow('Symbol "nonexistent" not found');
  });

  // ── unload ────────────────────────────────────────────────

  it('throws when unloading an unknown library', async () => {
    await expect(adapter.unload('/not/loaded')).rejects.toThrow(
      'Library not loaded',
    );
  });

  it('removes a loaded library on unload', async () => {
    const descriptor: FFILibraryDescriptor = {
      path: '/bin/echo',
      sourceLanguage: 'go',
      symbols: [],
    };

    await adapter.load(descriptor);
    expect(adapter.isLoaded('/bin/echo')).toBe(true);

    await adapter.unload('/bin/echo');
    expect(adapter.isLoaded('/bin/echo')).toBe(false);
    expect(adapter.listLibraries()).toEqual([]);
  });
});
