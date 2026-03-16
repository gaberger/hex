/**
 * HTTP-based Service Mesh Adapter
 *
 * Implements IServiceMeshPort for REST protocol endpoints.
 * Provides service registration, discovery, HTTP-based RPC calls with
 * retry + circuit breaker, polling-based subscriptions, and health checks.
 */

import type {
  IServiceMeshPort,
  ServiceEndpoint,
  ServiceCallOptions,
  ServiceCallResult,
  SerializedPayload,
} from '../../core/ports/cross-lang.js';

const DEFAULT_OPTIONS: ServiceCallOptions = {
  timeout: 5_000,
  retries: 2,
  circuitBreakerThreshold: 5,
};

interface CircuitState {
  consecutiveFailures: number;
  open: boolean;
}

export class HTTPServiceMeshAdapter implements IServiceMeshPort {
  private readonly endpoints = new Map<string, ServiceEndpoint[]>();
  private readonly circuits = new Map<string, CircuitState>();

  async register(endpoint: ServiceEndpoint): Promise<void> {
    if (endpoint.protocol !== 'rest') {
      throw new Error(
        `HTTPServiceMeshAdapter only supports protocol "rest", got "${endpoint.protocol}"`,
      );
    }
    const existing = this.endpoints.get(endpoint.serviceId) ?? [];
    const duplicate = existing.some(
      (e) => e.address === endpoint.address,
    );
    if (!duplicate) {
      existing.push(endpoint);
    }
    this.endpoints.set(endpoint.serviceId, existing);
    this.circuits.set(endpoint.serviceId, { consecutiveFailures: 0, open: false });
  }

  async discover(serviceId: string): Promise<ServiceEndpoint[]> {
    return this.endpoints.get(serviceId) ?? [];
  }

  async call<T>(
    serviceId: string,
    method: string,
    payload: SerializedPayload,
    options?: Partial<ServiceCallOptions>,
  ): Promise<ServiceCallResult<T>> {
    const opts: ServiceCallOptions = { ...DEFAULT_OPTIONS, ...options };
    const endpoints = this.endpoints.get(serviceId);
    if (!endpoints || endpoints.length === 0) {
      throw new Error(`No endpoints registered for service "${serviceId}"`);
    }

    const circuit = this.circuits.get(serviceId)!;
    if (circuit.open) {
      throw new Error(`Circuit breaker open for service "${serviceId}"`);
    }

    const endpoint = endpoints[0];
    const url = normalizeUrl(endpoint.address, method);

    let lastError: unknown;
    const attempts = 1 + opts.retries;

    for (let attempt = 0; attempt < attempts; attempt++) {
      const start = performance.now();
      try {
        const controller = new AbortController();
        const timer = setTimeout(() => controller.abort(), opts.timeout);

        const response = await fetch(url, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: decodePayload(payload),
          signal: controller.signal,
        });
        clearTimeout(timer);

        if (!response.ok) {
          throw new Error(`HTTP ${response.status}: ${response.statusText}`);
        }

        const value = (await response.json()) as T;
        const duration = performance.now() - start;

        // Reset circuit on success
        circuit.consecutiveFailures = 0;
        circuit.open = false;

        return { value, duration, handledBy: endpoint.address };
      } catch (err) {
        lastError = err;
        circuit.consecutiveFailures++;
        if (circuit.consecutiveFailures >= opts.circuitBreakerThreshold) {
          circuit.open = true;
          throw new Error(`Circuit breaker open for service "${serviceId}"`);
        }
      }
    }

    throw lastError;
  }

  async subscribe<T>(
    serviceId: string,
    subject: string,
    handler: (value: T) => void,
  ): Promise<() => void> {
    const endpoints = this.endpoints.get(serviceId);
    if (!endpoints || endpoints.length === 0) {
      throw new Error(`No endpoints registered for service "${serviceId}"`);
    }

    const endpoint = endpoints[0];
    const url = normalizeUrl(endpoint.address, subject);
    let active = true;

    const interval = setInterval(async () => {
      if (!active) return;
      try {
        const response = await fetch(url);
        if (response.ok) {
          const value = (await response.json()) as T;
          handler(value);
        }
      } catch {
        // Polling failures are silently ignored; next tick retries
      }
    }, 1_000);

    return () => {
      active = false;
      clearInterval(interval);
    };
  }

  async healthCheck(
    serviceId: string,
  ): Promise<{ healthy: boolean; latency: number }> {
    const endpoints = this.endpoints.get(serviceId);
    if (!endpoints || endpoints.length === 0) {
      return { healthy: false, latency: -1 };
    }

    const endpoint = endpoints[0];
    const start = performance.now();
    try {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), 3_000);
      const response = await fetch(endpoint.healthCheck, {
        signal: controller.signal,
      });
      clearTimeout(timer);
      const latency = performance.now() - start;
      return { healthy: response.ok, latency };
    } catch {
      const latency = performance.now() - start;
      return { healthy: false, latency };
    }
  }

  async deregister(serviceId: string): Promise<void> {
    this.endpoints.delete(serviceId);
    this.circuits.delete(serviceId);
  }
}

// ── Helpers ────────────────────────────────────────────────

function normalizeUrl(address: string, path: string): string {
  const base = address.startsWith('http') ? address : `http://${address}`;
  const sep = base.endsWith('/') ? '' : '/';
  const cleanPath = path.startsWith('/') ? path.slice(1) : path;
  return `${base}${sep}${cleanPath}`;
}

function decodePayload(payload: SerializedPayload): string {
  return new TextDecoder().decode(payload.data);
}
