import { describe, expect, it, mock } from "bun:test";
import type { ChatAnswer, DocContent, DocEntry, SystemSnapshot } from "../../src/core/domain/entities.js";
import type {
  IChatHistoryStore,
  IChatModel,
  IDocsReader,
  IKnowledgeIndex,
  ISystemStatsReader,
} from "../../src/core/ports/index.js";
import { DashboardService } from "../../src/core/usecases/service.js";

const fakeSnapshot: SystemSnapshot = {
  hostname: "test-host",
  takenAt: "2026-01-01T00:00:00.000Z",
  cpu: { model: "test-cpu", cores: 4, loadAvg1: 0, loadAvg5: 0, loadAvg15: 0 },
  memory: { totalBytes: 100, freeBytes: 50, usedBytes: 50 },
  disk: { mount: "/", totalBytes: 100, usedBytes: 50, freeBytes: 50 },
  gpus: [],
};

const fakeEntries: DocEntry[] = [
  { collection: "benchmarks", relativePath: "a.md", name: "a", kind: "file", sizeBytes: 10, modifiedAt: "2026-01-01T00:00:00.000Z" },
];

const fakeContent: DocContent = { collection: "benchmarks", relativePath: "a.md", name: "a", markdown: "# hi" };

const fakeMatches = [{ entry: fakeEntries[0], excerpt: "hi", score: 0.9 }];

const fakeHistory: ChatAnswer[] = [
  { question: "old q", answer: "old a", sources: [], answeredAt: "2026-01-01T00:00:00.000Z" },
];

function makeService() {
  const stats: ISystemStatsReader = { read: mock(() => Promise.resolve(fakeSnapshot)) };
  const docs: IDocsReader = {
    list: mock(() => Promise.resolve(fakeEntries)),
    read: mock(() => Promise.resolve(fakeContent)),
    search: mock(() => Promise.resolve(fakeEntries)),
    collections: mock(() => ["benchmarks", "adrs"]),
  };
  const knowledge: IKnowledgeIndex = { search: mock(() => Promise.resolve(fakeMatches)) };
  const chatModel: IChatModel = { answer: mock(() => Promise.resolve("the answer")) };
  const history: IChatHistoryStore = {
    append: mock(() => Promise.resolve()),
    list: mock(() => Promise.resolve(fakeHistory)),
  };
  return {
    service: new DashboardService(stats, docs, knowledge, chatModel, history),
    stats,
    docs,
    knowledge,
    chatModel,
    history,
  };
}

describe("DashboardService", () => {
  it("delegates getOverview to the stats port", async () => {
    const { service, stats } = makeService();
    const result = await service.getOverview();
    expect(result).toEqual(fakeSnapshot);
    expect(stats.read).toHaveBeenCalledTimes(1);
  });

  it("delegates listDocs to the docs port", async () => {
    const { service, docs } = makeService();
    const result = await service.listDocs("benchmarks");
    expect(result).toEqual(fakeEntries);
    expect(docs.list).toHaveBeenCalledWith("benchmarks");
  });

  it("delegates readDoc to the docs port", async () => {
    const { service, docs } = makeService();
    const result = await service.readDoc("benchmarks", "a.md");
    expect(result).toEqual(fakeContent);
    expect(docs.read).toHaveBeenCalledWith("benchmarks", "a.md");
  });

  it("exposes collections from the docs port", () => {
    const { service } = makeService();
    expect(service.collections()).toEqual(["benchmarks", "adrs"]);
  });

  it("delegates searchDocs to the docs port", async () => {
    const { service, docs } = makeService();
    const result = await service.searchDocs("test");
    expect(result).toEqual(fakeEntries);
    expect(docs.search).toHaveBeenCalledWith("test");
  });

  it("askQuestion retrieves matches then asks the chat model, citing sources", async () => {
    const { service, knowledge, chatModel } = makeService();
    const result = await service.askQuestion("what is this?");
    expect(knowledge.search).toHaveBeenCalledWith("what is this?", 10);
    expect(chatModel.answer).toHaveBeenCalledWith("what is this?", [{ source: "[benchmarks] a", text: "hi" }]);
    expect(result).toEqual({
      question: "what is this?",
      answer: "the answer",
      answeredAt: expect.any(String),
      sources: [{ collection: "benchmarks", relativePath: "a.md", name: "a", score: 0.9 }],
    });
  });

  it("askQuestion appends the result to the history store", async () => {
    const { service, history } = makeService();
    const result = await service.askQuestion("what is this?");
    expect(history.append).toHaveBeenCalledWith(result);
  });

  it("delegates getChatHistory to the history store", async () => {
    const { service, history } = makeService();
    const result = await service.getChatHistory();
    expect(result).toEqual(fakeHistory);
    expect(history.list).toHaveBeenCalledTimes(1);
  });
});
