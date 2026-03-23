/**
 * Inference Discovery Port (ADR-026)
 *
 * Contract for discovering available local/remote inference endpoints.
 * Adapters may back this with SpacetimeDB subscriptions, static config,
 * or manual registration.
 *
 * Dependency: domain types only.
 */

/** Supported inference providers (local and cloud). */
export type InferenceProvider = 'ollama' | 'openai-compatible' | 'vllm' | 'llama-cpp' | 'openrouter';

/** Health status of an inference endpoint. */
export type EndpointStatus = 'healthy' | 'unhealthy' | 'unknown';

/** A discoverable inference endpoint. */
export interface InferenceEndpoint {
  id: string;
  url: string;
  provider: InferenceProvider;
  model: string;
  status: EndpointStatus;
  requiresAuth: boolean;
  /** Secret key name in ISecretsPort (empty if no auth required). */
  secretKey: string;
  healthCheckedAt: string;
}

export interface IInferenceDiscoveryPort {
  /** List all registered inference endpoints. */
  listEndpoints(): Promise<InferenceEndpoint[]>;

  /** List only healthy endpoints, optionally filtered by provider. */
  healthyEndpoints(provider?: InferenceProvider): Promise<InferenceEndpoint[]>;

  /** Register a new inference endpoint. */
  registerEndpoint(endpoint: Omit<InferenceEndpoint, 'status' | 'healthCheckedAt'>): Promise<void>;

  /** Remove an inference endpoint by ID. */
  removeEndpoint(id: string): Promise<void>;

  /** Trigger a health check on all registered endpoints. */
  healthCheckAll(): Promise<void>;
}
