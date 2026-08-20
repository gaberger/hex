import type { IEmbedder } from "../../core/ports/index.js";

export class OllamaEmbedder implements IEmbedder {
  constructor(
    private readonly baseUrl: string = "http://localhost:11434",
    private readonly model: string = "nomic-embed-text",
  ) {}

  async embed(text: string): Promise<number[]> {
    const res = await fetch(`${this.baseUrl}/api/embed`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ model: this.model, input: text }),
    });
    if (!res.ok) throw new Error(`ollama embed failed: ${res.status} ${await res.text()}`);
    const data = (await res.json()) as { embeddings: number[][] };
    return data.embeddings[0];
  }
}
