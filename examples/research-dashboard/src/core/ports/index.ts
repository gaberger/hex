import type { DocContent, DocEntry, SystemSnapshot } from "../domain/entities.js";

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

/** Driving port — implemented by the use case, called by the primary (HTTP) adapter. */
export interface IDashboardService {
  getOverview(): Promise<SystemSnapshot>;
  listDocs(collection: string): Promise<DocEntry[]>;
  readDoc(collection: string, relativePath: string): Promise<DocContent>;
  searchDocs(query: string): Promise<DocEntry[]>;
  collections(): string[];
}
