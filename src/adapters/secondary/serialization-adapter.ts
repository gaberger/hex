/**
 * Serialization Adapter — implements ISerializationPort
 *
 * Supports JSON format natively. Protobuf and MessagePack are declared
 * but not yet wired — supportsFormat() returns false for them, and
 * serialize/deserialize throw if called with an unsupported format.
 */

import type {
  ISerializationPort,
  SerializationFormat,
  SerializedPayload,
} from '../../core/ports/cross-lang.js';

export class SerializationAdapter implements ISerializationPort {
  private readonly typeRegistry = new Map<SerializationFormat, Set<string>>();

  private readonly encoder = new TextEncoder();
  private readonly decoder = new TextDecoder();

  // ── Public API ────────────────────────────────────────────

  async serialize<T>(
    value: T,
    format: SerializationFormat,
    typeName: string,
  ): Promise<SerializedPayload> {
    this.assertSupported(format);
    this.trackType(format, typeName);

    const json = JSON.stringify(value);
    const data = this.encoder.encode(json);

    return { format, data, typeName };
  }

  async deserialize<T>(payload: SerializedPayload): Promise<T> {
    this.assertSupported(payload.format);

    const json = this.decoder.decode(payload.data);
    return JSON.parse(json) as T;
  }

  supportsFormat(format: SerializationFormat): boolean {
    return format === 'json';
  }

  registeredTypes(format: SerializationFormat): string[] {
    const set = this.typeRegistry.get(format);
    return set ? [...set] : [];
  }

  // ── Registration ──────────────────────────────────────────

  registerType(typeName: string, format: SerializationFormat): void {
    this.trackType(format, typeName);
  }

  // ── Internals ─────────────────────────────────────────────

  private assertSupported(format: SerializationFormat): void {
    if (!this.supportsFormat(format)) {
      throw new Error(`Format not supported: ${format}`);
    }
  }

  private trackType(format: SerializationFormat, typeName: string): void {
    let set = this.typeRegistry.get(format);
    if (!set) {
      set = new Set<string>();
      this.typeRegistry.set(format, set);
    }
    set.add(typeName);
  }
}
