/**
 * WASM Bridge secondary adapter -- implements IWASMBridgePort.
 *
 * Uses the Node.js built-in WebAssembly API to load, call, and manage
 * WebAssembly modules compiled from Rust or Go (via wasm-bindgen / TinyGo).
 */
import { readFile } from 'node:fs/promises';
import { performance } from 'node:perf_hooks';
import type {
  IWASMBridgePort,
  WASMModuleDescriptor,
  WASMCallResult,
  SerializedPayload,
} from '../../core/ports/cross-lang.js';

export class WASMModuleNotFoundError extends Error {
  constructor(moduleName: string) {
    super(`WASM module not loaded: "${moduleName}"`);
    this.name = 'WASMModuleNotFoundError';
  }
}

export class WASMFunctionNotFoundError extends Error {
  constructor(moduleName: string, functionName: string) {
    super(`Function "${functionName}" not found in WASM module "${moduleName}"`);
    this.name = 'WASMFunctionNotFoundError';
  }
}

interface LoadedModule {
  descriptor: WASMModuleDescriptor;
  instance: WebAssembly.Instance;
}

/**
 * Derives a module name from a WASMModuleDescriptor source path.
 * Uses the filename without extension as the key.
 */
function deriveModuleName(source: string): string {
  const base = source.split('/').pop() ?? source;
  return base.replace(/\.wasm$/, '');
}

/**
 * Deserialize a SerializedPayload into a JS value suitable for WASM calls.
 * Currently supports JSON format; protobuf/messagepack can be added via
 * the ISerializationPort if needed.
 */
function deserializeArg(payload: SerializedPayload): unknown {
  if (payload.format !== 'json') {
    throw new Error(`Unsupported serialization format for WASM bridge: "${payload.format}"`);
  }
  const text = new TextDecoder().decode(payload.data);
  return JSON.parse(text) as unknown;
}

export class WASMBridgeAdapter implements IWASMBridgePort {
  private readonly modules = new Map<string, LoadedModule>();

  async load(descriptor: WASMModuleDescriptor): Promise<void> {
    const name = deriveModuleName(descriptor.source);
    const wasmBytes = await readFile(descriptor.source);

    const memory = new WebAssembly.Memory({
      initial: descriptor.memory.initial,
      maximum: descriptor.memory.maximum,
      shared: descriptor.memory.shared,
    });

    const importObject = {
      env: { memory },
    };

    const { instance } = await WebAssembly.instantiate(wasmBytes, importObject);

    this.modules.set(name, { descriptor, instance });
  }

  async call<T>(
    moduleName: string,
    functionName: string,
    args: SerializedPayload[],
  ): Promise<WASMCallResult<T>> {
    const loaded = this.modules.get(moduleName);
    if (!loaded) {
      throw new WASMModuleNotFoundError(moduleName);
    }

    const fn = loaded.instance.exports[functionName];
    if (typeof fn !== 'function') {
      throw new WASMFunctionNotFoundError(moduleName, functionName);
    }

    const deserializedArgs = args.map(deserializeArg);

    const memBefore = this.getMemoryBytes(loaded);
    const start = performance.now();
    const value = (fn as (...a: unknown[]) => T)(...deserializedArgs);
    const duration = performance.now() - start;
    const memAfter = this.getMemoryBytes(loaded);

    return {
      value,
      duration,
      memoryUsed: Math.max(0, memAfter - memBefore),
    };
  }

  isLoaded(moduleName: string): boolean {
    return this.modules.has(moduleName);
  }

  async unload(moduleName: string): Promise<void> {
    if (!this.modules.has(moduleName)) {
      throw new WASMModuleNotFoundError(moduleName);
    }
    this.modules.delete(moduleName);
  }

  listModules(): WASMModuleDescriptor[] {
    return [...this.modules.values()].map((m) => m.descriptor);
  }

  /** Read current memory size in bytes from the module's env.memory or exported memory. */
  private getMemoryBytes(loaded: LoadedModule): number {
    const mem =
      (loaded.instance.exports['memory'] as WebAssembly.Memory | undefined);
    if (mem?.buffer) {
      return mem.buffer.byteLength;
    }
    return 0;
  }
}
