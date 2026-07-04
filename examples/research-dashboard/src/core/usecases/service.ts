import type { ChatAnswer, DocContent, DocEntry, SystemSnapshot } from "../domain/entities.js";
import type {
  IChatHistoryStore,
  IChatModel,
  IDashboardService,
  IDocsReader,
  IKnowledgeIndex,
  ISystemStatsReader,
} from "../ports/index.js";

const TOP_K = 8;

export class DashboardService implements IDashboardService {
  constructor(
    private readonly stats: ISystemStatsReader,
    private readonly docs: IDocsReader,
    private readonly knowledge: IKnowledgeIndex,
    private readonly chatModel: IChatModel,
    private readonly history: IChatHistoryStore,
  ) {}

  getOverview(): Promise<SystemSnapshot> {
    return this.stats.read();
  }

  listDocs(collection: string): Promise<DocEntry[]> {
    return this.docs.list(collection);
  }

  readDoc(collection: string, relativePath: string): Promise<DocContent> {
    return this.docs.read(collection, relativePath);
  }

  searchDocs(query: string): Promise<DocEntry[]> {
    return this.docs.search(query);
  }

  collections(): string[] {
    return this.docs.collections();
  }

  getChatHistory(): Promise<ChatAnswer[]> {
    return this.history.list();
  }

  async askQuestion(question: string): Promise<ChatAnswer> {
    const matches = await this.knowledge.search(question, TOP_K);
    const answer = await this.chatModel.answer(
      question,
      matches.map((m) => ({ source: `[${m.entry.collection}] ${m.entry.name}`, text: m.excerpt })),
    );
    // A doc can contribute more than one chunk to the model's context (see IKnowledgeIndex), but
    // the source list shown to the user should list each doc once, at its best score.
    const bestPerDoc = new Map<string, (typeof matches)[number]>();
    for (const m of matches) {
      const key = `${m.entry.collection}/${m.entry.relativePath}`;
      const existing = bestPerDoc.get(key);
      if (!existing || m.score > existing.score) bestPerDoc.set(key, m);
    }
    const result: ChatAnswer = {
      question,
      answer,
      answeredAt: new Date().toISOString(),
      sources: [...bestPerDoc.values()]
        .sort((a, b) => b.score - a.score)
        .map((m) => ({
          collection: m.entry.collection,
          relativePath: m.entry.relativePath,
          name: m.entry.name,
          score: m.score,
        })),
    };
    await this.history.append(result);
    return result;
  }
}
