/**
 * LLM secondary adapter -- implements ILLMPort.
 *
 * Uses raw fetch() to call Anthropic or OpenAI chat completion APIs.
 * No external SDK dependencies.
 */
import type {
  ILLMPort,
  LLMResponse,
  Message,
  TokenBudget,
} from '../../core/ports/index.js';

type LLMProvider = 'anthropic' | 'openai' | 'ollama' | 'openai-compatible';

export interface LLMAdapterConfig {
  provider: LLMProvider;
  /** API key. Empty string is valid for local providers that don't require auth. */
  apiKey: string;
  model?: string;
  baseUrl?: string;
}

const DEFAULT_MODELS: Record<LLMProvider, string> = {
  anthropic: 'claude-sonnet-4-20250514',
  openai: 'gpt-4o',
  ollama: 'llama3.1:latest',
  'openai-compatible': 'default',
};

const DEFAULT_URLS: Record<LLMProvider, string> = {
  anthropic: 'https://api.anthropic.com',
  openai: 'https://api.openai.com',
  ollama: 'http://127.0.0.1:11434',
  'openai-compatible': 'http://127.0.0.1:8000',
};

export class LLMAdapter implements ILLMPort {
  private readonly baseUrl: string;
  private readonly model: string;

  constructor(private readonly config: LLMAdapterConfig) {
    this.baseUrl = config.baseUrl ?? DEFAULT_URLS[config.provider];
    this.model = config.model ?? DEFAULT_MODELS[config.provider];
  }

  async prompt(budget: TokenBudget, messages: Message[]): Promise<LLMResponse> {
    const { url, headers, body } = this.buildRequest(budget, messages, false);
    const res = await fetch(url, { method: 'POST', headers, body: JSON.stringify(body) });
    if (!res.ok) {
      const text = await res.text();
      throw new Error(`LLM request failed (${res.status}): ${text}`);
    }
    const json = await res.json() as Record<string, unknown>;
    return this.parseResponse(json);
  }

  async *streamPrompt(budget: TokenBudget, messages: Message[]): AsyncGenerator<string> {
    const { url, headers, body } = this.buildRequest(budget, messages, true);
    const res = await fetch(url, { method: 'POST', headers, body: JSON.stringify(body) });
    if (!res.ok) {
      const text = await res.text();
      throw new Error(`LLM stream failed (${res.status}): ${text}`);
    }
    if (!res.body) throw new Error('Response body is null');

    const decoder = new TextDecoder();
    const reader = res.body.getReader();
    let buffer = '';

    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      buffer += decoder.decode(value, { stream: true });
      const lines = buffer.split('\n');
      buffer = lines.pop() ?? '';
      for (const line of lines) {
        const chunk = this.parseSSELine(line);
        if (chunk) yield chunk;
      }
    }
    if (buffer.length > 0) {
      const chunk = this.parseSSELine(buffer);
      if (chunk) yield chunk;
    }
  }

  // ── Private helpers ───────────────────────────────────────

  private buildRequest(
    budget: TokenBudget,
    messages: Message[],
    stream: boolean,
  ): { url: string; headers: Record<string, string>; body: Record<string, unknown> } {
    if (this.config.provider === 'anthropic') {
      const system = messages.find((m) => m.role === 'system')?.content;
      const nonSystem = messages.filter((m) => m.role !== 'system');
      return {
        url: `${this.baseUrl}/v1/messages`,
        headers: {
          'Content-Type': 'application/json',
          'x-api-key': this.config.apiKey,
          'anthropic-version': '2023-06-01',
        },
        body: {
          model: this.model,
          max_tokens: budget.reservedForResponse,
          ...(system ? { system } : {}),
          messages: nonSystem.map((m) => ({ role: m.role, content: m.content })),
          stream,
        },
      };
    }
    // OpenAI-compatible (covers 'openai', 'ollama', 'openai-compatible')
    const headers: Record<string, string> = {
      'Content-Type': 'application/json',
    };
    // Only set Authorization header when an API key is provided.
    // Local providers (ollama) typically don't require auth.
    if (this.config.apiKey) {
      headers['Authorization'] = `Bearer ${this.config.apiKey}`;
    }
    return {
      url: `${this.baseUrl}/v1/chat/completions`,
      headers,
      body: {
        model: this.model,
        max_tokens: budget.reservedForResponse,
        messages: messages.map((m) => ({ role: m.role, content: m.content })),
        stream,
      },
    };
  }

  /** Whether this provider uses the OpenAI-compatible response format. */
  private get isOpenAIFormat(): boolean {
    return this.config.provider !== 'anthropic';
  }

  private parseResponse(json: Record<string, unknown>): LLMResponse {
    if (!this.isOpenAIFormat) {
      if (!Array.isArray(json.content) || typeof json.usage !== 'object' || json.usage === null) {
        throw new Error('Invalid Anthropic response: missing content array or usage object');
      }
      const content = json.content as Array<{ text: string }>;
      const usage = json.usage as { input_tokens: number; output_tokens: number };
      return {
        content: content.map((c) => c.text).join(''),
        tokenUsage: { input: usage.input_tokens, output: usage.output_tokens },
        model: (json.model as string) ?? this.model,
      };
    }
    if (!Array.isArray(json.choices) || typeof json.usage !== 'object' || json.usage === null) {
      throw new Error('Invalid OpenAI response: missing choices array or usage object');
    }
    const choices = json.choices as Array<{ message: { content: string } }>;
    const usage = json.usage as { prompt_tokens: number; completion_tokens: number };
    return {
      content: choices[0]?.message.content ?? '',
      tokenUsage: { input: usage.prompt_tokens, output: usage.completion_tokens },
      model: (json.model as string) ?? this.model,
    };
  }

  private parseSSELine(line: string): string | null {
    const trimmed = line.trim();
    if (!trimmed.startsWith('data: ') || trimmed === 'data: [DONE]') return null;
    try {
      const data = JSON.parse(trimmed.slice(6)) as Record<string, unknown>;
      if (!this.isOpenAIFormat) {
        if (data.type === 'content_block_delta') {
          const delta = data.delta as { text?: string };
          return delta.text ?? null;
        }
        return null;
      }
      const choices = data.choices as Array<{ delta: { content?: string } }>;
      return choices?.[0]?.delta.content ?? null;
    } catch {
      // SSE lines may be malformed during streaming — skip silently
      return null;
    }
  }
}
