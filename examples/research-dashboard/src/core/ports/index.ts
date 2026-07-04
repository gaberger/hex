import type { ChatAnswer, DocContent, DocEntry, SystemSnapshot } from "../domain/entities.js";

/** Driven port — implemented by a secondary adapter that reads live host stats. */
export interface ISystemStatsReader {
  read(): Promise<SystemSnapshot>;
}

/** Driven port — implemented by a secondary adapter that reads markdown off disk. */
export interface IDocsReader {
  list(collection: string): Promise<DocEntry[]>;
  read(collection: string, relativePath: string): Promise<DocContent>;
  search(query: string): Promise<DocEntry[]>;
  collections(): string[];
}

/** Driven port — implemented by a secondary adapter that calls a local embedding model. */
export interface IEmbedder {
  embed(text: string): Promise<number[]>;
}

/** Driven port — implemented by a secondary adapter that calls a local chat model. */
export interface IChatModel {
  answer(question: string, contextChunks: { source: string; text: string }[]): Promise<string>;
}

export interface KnowledgeMatch {
  entry: DocEntry;
  excerpt: string;
  score: number;
}

/** Driven port — semantic search over the knowledge base (embeddings-backed). */
export interface IKnowledgeIndex {
  search(query: string, topK: number): Promise<KnowledgeMatch[]>;
}

/** Driven port — implemented by a secondary adapter that persists chat Q&A across restarts. */
export interface IChatHistoryStore {
  append(entry: ChatAnswer): Promise<void>;
  list(): Promise<ChatAnswer[]>;
}

/** Driving port — implemented by the use case, called by the primary (HTTP) adapter. */
export interface IDashboardService {
  getOverview(): Promise<SystemSnapshot>;
  listDocs(collection: string): Promise<DocEntry[]>;
  readDoc(collection: string, relativePath: string): Promise<DocContent>;
  searchDocs(query: string): Promise<DocEntry[]>;
  collections(): string[];
  askQuestion(question: string): Promise<ChatAnswer>;
  getChatHistory(): Promise<ChatAnswer[]>;
}
