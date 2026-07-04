import type { IChatModel } from "../../core/ports/index.js";

const SYSTEM_PROMPT = `You are a research assistant answering questions about an internal
knowledge base (ADRs, specs, benchmarks, analysis, guides). Answer ONLY using the provided
context excerpts below. If the context doesn't contain the answer, say so plainly instead of
guessing. Cite which source(s) you used by name.`;

export class OllamaChatModel implements IChatModel {
  constructor(
    private readonly baseUrl: string = "http://localhost:11434",
    private readonly model: string = "devstral-small-2:24b",
  ) {}

  async answer(question: string, contextChunks: { source: string; text: string }[]): Promise<string> {
    const context = contextChunks
      .map((c, i) => `[${i + 1}] Source: ${c.source}\n${c.text}`)
      .join("\n\n---\n\n");

    const res = await fetch(`${this.baseUrl}/api/chat`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        model: this.model,
        stream: false,
        think: false,
        messages: [
          { role: "system", content: SYSTEM_PROMPT },
          { role: "user", content: `Context:\n\n${context}\n\nQuestion: ${question}` },
        ],
      }),
    });
    if (!res.ok) throw new Error(`ollama chat failed: ${res.status} ${await res.text()}`);
    const data = (await res.json()) as { message: { content: string } };
    return data.message.content;
  }
}
