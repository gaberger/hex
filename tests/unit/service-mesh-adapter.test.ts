import { describe, it, expect, beforeEach } from 'bun:test';
import { HTTPServiceMeshAdapter } from '../../src/adapters/secondary/service-mesh-adapter.js';
import type { ServiceEndpoint, SerializedPayload } from '../../src/core/ports/cross-lang.js';

const makeEndpoint = (overrides?: Partial<ServiceEndpoint>): ServiceEndpoint => ({
  serviceId: 'test-svc',
  language: 'typescript',
  protocol: 'rest',
  address: 'http://localhost:9999',
  healthCheck: 'http://localhost:9999/health',
  ...overrides,
});

const makePayload = (obj: unknown): SerializedPayload => ({
  format: 'json',
  data: new TextEncoder().encode(JSON.stringify(obj)),
  typeName: 'TestPayload',
});

describe('HTTPServiceMeshAdapter', () => {
  let adapter: HTTPServiceMeshAdapter;

  beforeEach(() => {
    adapter = new HTTPServiceMeshAdapter();
  });

  // ── register / discover / deregister lifecycle ──────────

  describe('register / discover / deregister', () => {
    it('registers and discovers an endpoint', async () => {
      const ep = makeEndpoint();
      await adapter.register(ep);
      const found = await adapter.discover('test-svc');
      expect(found).toHaveLength(1);
      expect(found[0].address).toBe('http://localhost:9999');
    });

    it('registers multiple endpoints for the same serviceId', async () => {
      await adapter.register(makeEndpoint({ address: 'http://localhost:9001' }));
      await adapter.register(makeEndpoint({ address: 'http://localhost:9002' }));
      const found = await adapter.discover('test-svc');
      expect(found).toHaveLength(2);
    });

    it('deduplicates endpoints with the same address', async () => {
      const ep = makeEndpoint();
      await adapter.register(ep);
      await adapter.register(ep);
      const found = await adapter.discover('test-svc');
      expect(found).toHaveLength(1);
    });

    it('deregisters a service', async () => {
      await adapter.register(makeEndpoint());
      await adapter.deregister('test-svc');
      const found = await adapter.discover('test-svc');
      expect(found).toHaveLength(0);
    });

    it('deregister is idempotent for unknown services', async () => {
      await expect(adapter.deregister('unknown')).resolves.toBeUndefined();
    });
  });

  // ── discover ────────────────────────────────────────────

  describe('discover', () => {
    it('returns empty array for unknown serviceId', async () => {
      const found = await adapter.discover('nonexistent');
      expect(found).toEqual([]);
    });
  });

  // ── register validation ─────────────────────────────────

  describe('register validation', () => {
    it('throws for non-rest protocol', async () => {
      const ep = makeEndpoint({ protocol: 'grpc' });
      await expect(adapter.register(ep)).rejects.toThrow(
        'only supports protocol "rest"',
      );
    });

    it('throws for nats protocol', async () => {
      const ep = makeEndpoint({ protocol: 'nats' });
      await expect(adapter.register(ep)).rejects.toThrow(
        'only supports protocol "rest"',
      );
    });
  });

  // ── call ────────────────────────────────────────────────

  describe('call', () => {
    it('throws for unregistered service', async () => {
      const payload = makePayload({ x: 1 });
      await expect(
        adapter.call('no-such-svc', 'doStuff', payload),
      ).rejects.toThrow('No endpoints registered for service "no-such-svc"');
    });

    it('throws on network failure after retries exhausted', async () => {
      await adapter.register(makeEndpoint());
      const payload = makePayload({ x: 1 });
      // localhost:9999 is not running, so fetch will fail
      await expect(
        adapter.call('test-svc', 'action', payload, { retries: 0, timeout: 500 }),
      ).rejects.toThrow();
    });

    it('opens circuit breaker after threshold failures', async () => {
      await adapter.register(makeEndpoint());
      const payload = makePayload({});

      // Exhaust circuit breaker (threshold default = 5)
      for (let i = 0; i < 5; i++) {
        try {
          await adapter.call('test-svc', 'fail', payload, {
            retries: 0,
            timeout: 200,
          });
        } catch {
          // expected
        }
      }

      // Next call should get circuit breaker error
      await expect(
        adapter.call('test-svc', 'fail', payload, { retries: 0, timeout: 200 }),
      ).rejects.toThrow('Circuit breaker open');
    });
  });

  // ── healthCheck ─────────────────────────────────────────

  describe('healthCheck', () => {
    it('returns unhealthy with latency -1 for unknown service', async () => {
      const result = await adapter.healthCheck('unknown');
      expect(result.healthy).toBe(false);
      expect(result.latency).toBe(-1);
    });

    it('returns unhealthy when endpoint is not reachable', async () => {
      await adapter.register(makeEndpoint());
      const result = await adapter.healthCheck('test-svc');
      expect(result.healthy).toBe(false);
      expect(result.latency).toBeGreaterThanOrEqual(0);
    });
  });

  // ── subscribe ───────────────────────────────────────────

  describe('subscribe', () => {
    it('throws for unregistered service', async () => {
      await expect(
        adapter.subscribe('no-svc', 'topic', () => {}),
      ).rejects.toThrow('No endpoints registered');
    });

    it('returns an unsubscribe function', async () => {
      await adapter.register(makeEndpoint());
      const unsub = await adapter.subscribe('test-svc', 'events', () => {});
      expect(typeof unsub).toBe('function');
      // Clean up the polling interval
      unsub();
    });
  });
});
