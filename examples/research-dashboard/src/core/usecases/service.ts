import type { DocContent, DocEntry, SystemSnapshot } from "../domain/entities.js";
import type { IDashboardService, IDocsReader, ISystemStatsReader } from "../ports/index.js";

export class DashboardService implements IDashboardService {
  constructor(
    private readonly stats: ISystemStatsReader,
    private readonly docs: IDocsReader,
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
}
