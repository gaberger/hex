/**
 * hex-intf Cross-Language Communication Ports
 *
 * Port interfaces for communication across language boundaries (TS <-> Go, TS <-> Rust, Go <-> Rust).
 * These ports abstract the serialization, bridging, and discovery mechanisms so that
 * adapters can be swapped between WASM, FFI, gRPC, and REST without changing domain logic.
 */

import type { Language, CodeUnit } from './index.js';

// ─── Value Objects ───────────────────────────────────────

export type SerializationFormat = 'json' | 'protobuf' | 'messagepack';

export type BridgeType = 'wasm' | 'ffi' | 'grpc' | 'rest' | 'nats';

export interface SerializedPayload {
  format: SerializationFormat;
  data: Uint8Array;
  /** Original TypeScript type name for deserialization routing */
  typeName: string;
}

export interface WASMModuleDescriptor {
  /** Path to the .wasm file or package name */
  source: string;
  /** Language the module was compiled from */
  sourceLanguage: Language;
  /** Exported function names available for calling */
  exports: string[];
  /** Memory configuration */
  memory: {
    initial: number; // pages (64KB each)
    maximum?: number;
    shared?: boolean;
  };
}

export interface WASMCallResult<T = unknown> {
  value: T;
  /** Execution time in milliseconds */
  duration: number;
  /** Memory used in bytes */
  memoryUsed: number;
}

export interface FFILibraryDescriptor {
  /** Path to the shared library (.so, .dylib, .dll) */
  path: string;
  /** Language the library was compiled from */
  sourceLanguage: Language;
  /** Exported symbol names */
  symbols: FFISymbol[];
}

export interface FFISymbol {
  name: string;
  /** C ABI function signature */
  signature: string;
  /** Whether this function allocates memory the caller must free */
  allocates: boolean;
}

export interface FFICallResult<T = unknown> {
  value: T;
  duration: number;
}

export interface ServiceEndpoint {
  /** Service identifier (e.g., 'ast-engine', 'compiler') */
  serviceId: string;
  /** Language the service is implemented in */
  language: Language;
  /** Connection protocol */
  protocol: 'grpc' | 'rest' | 'nats';
  /** Host and port (e.g., 'localhost:50051') or NATS subject prefix */
  address: string;
  /** Health check endpoint or subject */
  healthCheck: string;
}

export interface ServiceCallOptions {
  /** Timeout in milliseconds */
  timeout: number;
  /** Number of retry attempts on transient failure */
  retries: number;
  /** Circuit breaker: stop calling after this many consecutive failures */
  circuitBreakerThreshold: number;
}

export interface ServiceCallResult<T = unknown> {
  value: T;
  duration: number;
  /** Which service instance handled the request */
  handledBy: string;
}

export type SchemaFormat = 'openapi' | 'protobuf' | 'jsonschema';

export interface SchemaDefinition {
  format: SchemaFormat;
  /** Raw schema content */
  content: string;
  /** Version string for cache invalidation */
  version: string;
}

export interface TypeMapping {
  /** Type name in the schema (e.g., protobuf message name) */
  schemaName: string;
  /** Corresponding type in each language */
  languageTypes: Partial<Record<Language, string>>;
}

export interface SchemaValidationResult {
  valid: boolean;
  errors: SchemaValidationError[];
}

export interface SchemaValidationError {
  path: string;
  message: string;
  expected: string;
  actual: string;
}

// ─── Output Ports (Secondary / Driven) ───────────────────

/**
 * Serialization across language boundaries.
 *
 * Adapters: JsonSerializationAdapter, ProtobufSerializationAdapter, MessagePackSerializationAdapter
 */
export interface ISerializationPort {
  /** Serialize a TypeScript value into a format suitable for cross-language transfer */
  serialize<T>(value: T, format: SerializationFormat, typeName: string): Promise<SerializedPayload>;

  /** Deserialize a payload received from another language back into a TypeScript value */
  deserialize<T>(payload: SerializedPayload): Promise<T>;

  /** Check if a format is available (e.g., protobuf requires generated code) */
  supportsFormat(format: SerializationFormat): boolean;

  /** List all registered type mappings for a given format */
  registeredTypes(format: SerializationFormat): string[];
}

/**
 * Load and call WebAssembly modules compiled from Rust or Go.
 *
 * Adapters: WasmBindgenBridgeAdapter (Rust), TinyGoBridgeAdapter (Go)
 */
export interface IWASMBridgePort {
  /** Load a WASM module and prepare it for calling */
  load(descriptor: WASMModuleDescriptor): Promise<void>;

  /** Call an exported WASM function with serialized arguments, returning a deserialized result */
  call<T>(moduleName: string, functionName: string, args: SerializedPayload[]): Promise<WASMCallResult<T>>;

  /** Check if a module is loaded and ready */
  isLoaded(moduleName: string): boolean;

  /** Unload a module and free its memory */
  unload(moduleName: string): Promise<void>;

  /** List all currently loaded modules */
  listModules(): WASMModuleDescriptor[];
}

/**
 * Call native libraries via Foreign Function Interface.
 *
 * Adapters: NapiRsFFIAdapter (Rust via napi-rs for Node), CGoFFIAdapter (Go via CGo)
 */
export interface IFFIPort {
  /** Load a native library and register its symbols */
  load(descriptor: FFILibraryDescriptor): Promise<void>;

  /** Call a native function by symbol name with serialized arguments */
  call<T>(libraryPath: string, symbolName: string, args: SerializedPayload[]): Promise<FFICallResult<T>>;

  /** Check if a library is loaded */
  isLoaded(libraryPath: string): boolean;

  /** Unload a library */
  unload(libraryPath: string): Promise<void>;

  /** List all loaded libraries and their symbols */
  listLibraries(): FFILibraryDescriptor[];
}

/**
 * Discover and call services implemented in other languages.
 *
 * Adapters: GRPCServiceMeshAdapter, NATSServiceMeshAdapter, HTTPServiceMeshAdapter
 */
export interface IServiceMeshPort {
  /** Register a service endpoint for discovery */
  register(endpoint: ServiceEndpoint): Promise<void>;

  /** Discover available endpoints for a service */
  discover(serviceId: string): Promise<ServiceEndpoint[]>;

  /** Call a service method with automatic serialization and deserialization */
  call<T>(
    serviceId: string,
    method: string,
    payload: SerializedPayload,
    options?: Partial<ServiceCallOptions>,
  ): Promise<ServiceCallResult<T>>;

  /** Subscribe to events from a service (NATS pub/sub or gRPC streaming) */
  subscribe<T>(serviceId: string, subject: string, handler: (value: T) => void): Promise<() => void>;

  /** Health check a specific service */
  healthCheck(serviceId: string): Promise<{ healthy: boolean; latency: number }>;

  /** Deregister a service endpoint */
  deregister(serviceId: string): Promise<void>;
}

/**
 * Shared type definitions that span language boundaries.
 *
 * Adapters: OpenAPISchemaAdapter, ProtobufSchemaAdapter, JsonSchemaAdapter
 */
export interface ISchemaPort {
  /** Load a schema definition from file or string */
  load(schema: SchemaDefinition): Promise<void>;

  /** Validate a value against a named type in the schema */
  validate<T>(typeName: string, value: T): Promise<SchemaValidationResult>;

  /** Get the type mapping for a schema type across all supported languages */
  getTypeMapping(typeName: string): Promise<TypeMapping>;

  /** List all types defined in loaded schemas */
  listTypes(): Promise<string[]>;

  /** Generate TypeScript type declarations from the schema (for code generation pipelines) */
  generateTypes(targetLanguage: Language): Promise<CodeUnit>;

  /** Compare two schema versions and report breaking changes */
  diffSchemas(before: SchemaDefinition, after: SchemaDefinition): Promise<SchemaBreakingChange[]>;
}

export interface SchemaBreakingChange {
  typeName: string;
  field: string;
  kind: 'removed' | 'type-changed' | 'required-added' | 'enum-value-removed';
  description: string;
}
