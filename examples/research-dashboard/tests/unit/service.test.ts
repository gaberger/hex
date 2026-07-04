import { describe, expect, it, mock } from "bun:test";
import type { DocContent, DocEntry, SystemSnapshot } from "../../src/core/domain/entities.js";
import type { IDocsReader, ISystemStatsReader } from "../../src/core/ports/index.js";
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

function makeService() {
  const stats: ISystemStatsReader = { read: mock(() => Promise.resolve(fakeSnapshot)) };
  const docs: IDocsReader = {
    list: mock(() => Promise.resolve(fakeEntries)),
    read: mock(() => Promise.resolve(fakeContent)),
    search: mock(() => Promise.resolve(fakeEntries)),
    collections: mock(() => ["benchmarks", "adrs"]),
  };
  return { service: new DashboardService(stats, docs), stats, docs };
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
});
