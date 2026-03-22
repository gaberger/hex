/**
 * rest-client.ts — Singleton REST client secondary adapter (ADR-056).
 *
 * All HTTP calls to hex-nexus go through this service.
 * Stores and components import `restClient` instead of calling fetch() directly.
 */
import type { IRestClient } from '../types/services';

class RestClient implements IRestClient {
  private getHeaders(hasBody: boolean): HeadersInit {
    const headers: Record<string, string> = {};
    if (hasBody) {
      headers['Content-Type'] = 'application/json';
    }
    // Inject auth token if available
    const token = localStorage.getItem('hex-auth-token');
    if (token) {
      headers['Authorization'] = `Bearer ${token}`;
    }
    return headers;
  }

  async get<T = any>(path: string): Promise<T> {
    const res = await fetch(path, {
      headers: this.getHeaders(false),
    });
    if (!res.ok) {
      const body = await res.json().catch(() => ({ error: res.statusText }));
      throw new Error(body.error ?? `HTTP ${res.status}`);
    }
    return res.json();
  }

  async post<T = any>(path: string, body?: unknown): Promise<T> {
    const res = await fetch(path, {
      method: 'POST',
      headers: this.getHeaders(body !== undefined),
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
    if (!res.ok) {
      const errBody = await res.json().catch(() => ({ error: res.statusText }));
      throw new Error(errBody.error ?? `HTTP ${res.status}`);
    }
    return res.json();
  }

  async put<T = any>(path: string, body?: unknown): Promise<T> {
    const res = await fetch(path, {
      method: 'PUT',
      headers: this.getHeaders(body !== undefined),
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
    if (!res.ok) {
      const errBody = await res.json().catch(() => ({ error: res.statusText }));
      throw new Error(errBody.error ?? `HTTP ${res.status}`);
    }
    return res.json();
  }

  async patch<T = any>(path: string, body?: unknown): Promise<T> {
    const res = await fetch(path, {
      method: 'PATCH',
      headers: this.getHeaders(body !== undefined),
      body: body !== undefined ? JSON.stringify(body) : undefined,
    });
    if (!res.ok) {
      const errBody = await res.json().catch(() => ({ error: res.statusText }));
      throw new Error(errBody.error ?? `HTTP ${res.status}`);
    }
    return res.json();
  }

  async delete(path: string): Promise<void> {
    const res = await fetch(path, {
      method: 'DELETE',
      headers: this.getHeaders(false),
    });
    if (!res.ok) {
      const body = await res.json().catch(() => ({ error: res.statusText }));
      throw new Error(body.error ?? `HTTP ${res.status}`);
    }
  }
}

/** Singleton REST client. */
export const restClient: IRestClient = new RestClient();
