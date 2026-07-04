import type { DocEntry } from "../../core/domain/entities.js";
import type { IDocsReader, IEmbedder, IKnowledgeIndex, KnowledgeMatch } from "../../core/ports/index.js";

interface ChunkRecord {
  entry: DocEntry;
  text: string;
  embedding: number[];
}

function chunkMarkdown(text: string, maxChars = 1500): string[] {
  const paragraphs = text.split(/\n{2,}/);
  const chunks: string[] = [];
  let current = "";
  for (const p of paragraphs) {
    if (current.length > 0 && current.length + p.length > maxChars) {
      chunks.push(current.trim());
      current = p;
    } else {
      current = current ? `${current}\n\n${p}` : p;
    }
  }
  if (current.trim()) chunks.push(current.trim());
  return chunks.length > 0 ? chunks : [text];
}

function cosineSimilarity(a: number[], b: number[]): number {
  let dot = 0;
  let normA = 0;
  let normB = 0;
  for (let i = 0; i < a.length; i++) {
    dot += a[i] * b[i];
    normA += a[i] * a[i];
    normB += b[i] * b[i];
  }
  return dot / (Math.sqrt(normA) * Math.sqrt(normB));
}

/** In-memory, no-persistence semantic index — rebuilt on every process start. */
export class EmbeddingKnowledgeIndex implements IKnowledgeIndex {
  private buildPromise: Promise<ChunkRecord[]> | null = null;

  constructor(
    private readonly embedder: IEmbedder,
    private readonly docs: IDocsReader,
  ) {}

  /** Kicks off indexing without blocking the caller; safe to call more than once. */
  ensureIndexed(): void {
    void this.getRecords();
  }

  private getRecords(): Promise<ChunkRecord[]> {
    if (!this.buildPromise) this.buildPromise = this.build();
    return this.buildPromise;
  }

  private async build(): Promise<ChunkRecord[]> {
    const records: ChunkRecord[] = [];
    for (const collection of this.docs.collections()) {
      const entries = await this.docs.list(collection);
      for (const entry of entries) {
        const content = await this.docs.read(collection, entry.relativePath);
        for (const text of chunkMarkdown(content.markdown)) {
          records.push({ entry, text, embedding: await this.embedder.embed(text) });
        }
      }
    }
    console.log(`knowledge index built: ${records.length} chunks`);
    return records;
  }

  async search(query: string, topK: number): Promise<KnowledgeMatch[]> {
    const records = await this.getRecords();
    const queryEmbedding = await this.embedder.embed(query);
    const scored = records
      .map((r) => ({ record: r, score: cosineSimilarity(queryEmbedding, r.embedding) }))
      .sort((a, b) => b.score - a.score);

    // Allow up to 2 chunks per document — a single long doc (e.g. a 24KB test plan) can have its
    // most relevant section several paragraphs past whichever chunk scores highest on its own.
    const perDocCount = new Map<string, number>();
    const results: KnowledgeMatch[] = [];
    for (const { record, score } of scored) {
      const key = `${record.entry.collection}/${record.entry.relativePath}`;
      const count = perDocCount.get(key) ?? 0;
      if (count >= 2) continue;
      perDocCount.set(key, count + 1);
      results.push({ entry: record.entry, excerpt: record.text, score });
      if (results.length >= topK) break;
    }
    return results;
  }
}
