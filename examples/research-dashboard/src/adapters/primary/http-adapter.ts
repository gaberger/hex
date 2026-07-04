import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import express, { type Express } from "express";
import { marked } from "marked";
import type { IDashboardService } from "../../core/ports/index.js";
import { docListFragment, layout, systemFragment } from "./templates.js";

const publicDir = join(dirname(fileURLToPath(import.meta.url)), "public");

export function createHttpApp(service: IDashboardService): Express {
  const app = express();
  app.use(express.static(publicDir));

  app.get("/", async (_req, res) => {
    const collections = service.collections();
    const [snapshot, recentByCollection] = await Promise.all([
      service.getOverview(),
      Promise.all(collections.map((c) => service.listDocs(c))),
    ]);
    const recent = recentByCollection
      .flat()
      .sort((a, b) => b.modifiedAt.localeCompare(a.modifiedAt))
      .slice(0, 10);

    const body = `
      <h1>research-dashboard</h1>
      <div hx-get="/fragments/system" hx-trigger="load, every 5s" hx-swap="innerHTML">
        ${systemFragment(snapshot)}
      </div>
      <h2>Recently updated docs</h2>
      <div class="card">${docListFragment(recent, true)}</div>
    `;
    res.send(layout({ title: "home", collections, body }));
  });

  app.get("/fragments/system", async (_req, res) => {
    res.send(systemFragment(await service.getOverview()));
  });

  app.get("/api/system", async (_req, res) => {
    res.json(await service.getOverview());
  });

  app.get("/system", async (_req, res) => {
    const s = await service.getOverview();
    const body = `<h1>system overview</h1><div hx-get="/fragments/system" hx-trigger="load, every 5s" hx-swap="innerHTML">${systemFragment(s)}</div>`;
    res.send(layout({ title: "system", collections: service.collections(), body }));
  });

  app.get("/search", async (req, res) => {
    const q = String(req.query.q ?? "");
    if (!q.trim()) return res.send("");
    const results = await service.searchDocs(q);
    res.send(docListFragment(results, true));
  });

  app.get("/api/docs/:collection", async (req, res) => {
    res.json(await service.listDocs(req.params.collection));
  });

  app.get("/docs/:collection", async (req, res) => {
    const collection = req.params.collection;
    const entries = await service.listDocs(collection);
    const body = `<h1>${collection}</h1><p class="muted">${entries.length} docs</p><div class="card">${docListFragment(entries)}</div>`;
    res.send(layout({ title: collection, collections: service.collections(), activeCollection: collection, body }));
  });

  app.get("/api/docs/:collection/*path", async (req, res) => {
    const relativePath = (req.params.path as unknown as string[]).join("/");
    res.json(await service.readDoc(req.params.collection, relativePath));
  });

  app.get("/docs/:collection/*path", async (req, res) => {
    const collection = req.params.collection;
    const relativePath = (req.params.path as unknown as string[]).join("/");
    const doc = await service.readDoc(collection, relativePath);
    const body = `<p class="muted"><a href="/docs/${collection}">&larr; back to ${collection}</a></p><h1>${doc.name}</h1><div class="card">${await marked.parse(doc.markdown)}</div>`;
    res.send(layout({ title: doc.name, collections: service.collections(), activeCollection: collection, body }));
  });

  app.use((err: Error, _req: express.Request, res: express.Response, _next: express.NextFunction) => {
    res.status(404).send(layout({ title: "not found", collections: service.collections(), body: `<p>${err.message}</p>` }));
  });

  return app;
}
