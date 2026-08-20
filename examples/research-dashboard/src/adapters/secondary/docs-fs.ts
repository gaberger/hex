import { promises as fs } from "node:fs";
import { basename, resolve, sep } from "node:path";
import type { DocContent, DocEntry } from "../../core/domain/entities.js";
import type { IDocsReader } from "../../core/ports/index.js";

/** Resolves `relativePath` inside `baseDir`, rejecting any traversal outside it. */
async function safePath(baseDir: string, relativePath: string): Promise<string> {
  const resolvedBase = await fs.realpath(baseDir);
  const candidate = resolve(resolvedBase, relativePath);
  if (candidate !== resolvedBase && !candidate.startsWith(resolvedBase + sep)) {
    throw new Error(`path escapes collection root: ${relativePath}`);
  }
  const real = await fs.realpath(candidate);
  if (real !== resolvedBase && !real.startsWith(resolvedBase + sep)) {
    throw new Error(`path escapes collection root via symlink: ${relativePath}`);
  }
  return real;
}

export class DocsFsReader implements IDocsReader {
  constructor(private readonly baseDirs: Record<string, string>) {}

  collections(): string[] {
    return Object.keys(this.baseDirs);
  }

  private baseDirFor(collection: string): string {
    const dir = this.baseDirs[collection];
    if (!dir) throw new Error(`unknown collection: ${collection}`);
    return dir;
  }

  async list(collection: string): Promise<DocEntry[]> {
    const baseDir = this.baseDirFor(collection);
    const names = await fs.readdir(baseDir);
    const entries = await Promise.all(
      names
        .filter((name) => name.endsWith(".md"))
        .map(async (name): Promise<DocEntry> => {
          const stat = await fs.stat(resolve(baseDir, name));
          return {
            collection,
            relativePath: name,
            name: basename(name, ".md"),
            kind: "file",
            sizeBytes: stat.size,
            modifiedAt: stat.mtime.toISOString(),
          };
        }),
    );
    return entries.sort((a, b) => b.modifiedAt.localeCompare(a.modifiedAt));
  }

  async read(collection: string, relativePath: string): Promise<DocContent> {
    const baseDir = this.baseDirFor(collection);
    const fullPath = await safePath(baseDir, relativePath);
    const markdown = await fs.readFile(fullPath, "utf-8");
    return { collection, relativePath, name: basename(relativePath, ".md"), markdown };
  }

  async search(query: string): Promise<DocEntry[]> {
    const needle = query.trim().toLowerCase();
    if (!needle) return [];
    const perCollection = await Promise.all(this.collections().map((c) => this.list(c)));
    return perCollection.flat().filter((e) => e.name.toLowerCase().includes(needle));
  }
}
