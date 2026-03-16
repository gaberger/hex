/**
 * FFI Adapter — implements IFFIPort
 *
 * Calls native binaries via child_process.execFile (no shell injection).
 * The calling convention: execFile(libraryPath, [symbolName, ...jsonArgs]).
 * The native binary receives the symbol name as first arg and JSON-encoded
 * serialized payloads as subsequent args. Stdout is parsed as JSON.
 */

import { createRequire } from 'node:module';
const _require = createRequire(import.meta.url);
const { execFile: execFileCb } = _require('node:child_process');
import { access, constants } from 'node:fs/promises';
import { promisify } from 'node:util';

import type {
  IFFIPort,
  FFILibraryDescriptor,
  FFICallResult,
  SerializedPayload,
} from '../../core/ports/cross-lang.js';

const execFile = promisify(execFileCb);

export class FFIAdapter implements IFFIPort {
  private readonly libraries = new Map<string, FFILibraryDescriptor>();

  private readonly decoder = new TextDecoder();

  // ── Public API ────────────────────────────────────────────

  async load(descriptor: FFILibraryDescriptor): Promise<void> {
    await access(descriptor.path, constants.X_OK).catch(() => {
      throw new Error(`Library not found or not executable: ${descriptor.path}`);
    });

    this.libraries.set(descriptor.path, descriptor);
  }

  async call<T>(
    libraryPath: string,
    symbolName: string,
    args: SerializedPayload[],
  ): Promise<FFICallResult<T>> {
    const descriptor = this.libraries.get(libraryPath);
    if (!descriptor) {
      throw new Error(`Library not loaded: ${libraryPath}`);
    }

    const symbol = descriptor.symbols.find((s) => s.name === symbolName);
    if (!symbol) {
      throw new Error(
        `Symbol "${symbolName}" not found in library ${libraryPath}`,
      );
    }

    const jsonArgs = args.map((a) => {
      const decoded = this.decoder.decode(a.data);
      return decoded;
    });

    const start = performance.now();
    const { stdout } = await execFile(libraryPath, [symbolName, ...jsonArgs]);
    const duration = performance.now() - start;

    const value = JSON.parse(stdout) as T;
    return { value, duration };
  }

  isLoaded(libraryPath: string): boolean {
    return this.libraries.has(libraryPath);
  }

  async unload(libraryPath: string): Promise<void> {
    if (!this.libraries.delete(libraryPath)) {
      throw new Error(`Library not loaded: ${libraryPath}`);
    }
  }

  listLibraries(): FFILibraryDescriptor[] {
    return [...this.libraries.values()];
  }
}
