import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import express, { type Express } from "express";
import { marked } from "marked";
import type { IDashboardService } from "../../core/ports/index.js";
import { chatAnswerFragment, docListFragment, layout, systemFragment } from "./templates.js";

const publicDir = join(dirname(fileURLToPath(import.meta.url)), "public");

export function createHttpApp(service: IDashboardService): Express {
  const app = express();
  app.use(express.static(publicDir));
  app.use(express.urlencoded({ extended: true }));

  app.get("/", async (_req, res) => {
    const history = await service.getChatHistory();
    const historyHtml = history
      .slice()
      .reverse()
      .map((h) => chatAnswerFragment(h))
      .join("");
    const body = `
      <h1>ask the knowledge base</h1>
      <p class="muted">Semantic search over every doc collection, answered by a local model. First
      question after a restart waits for the index to finish building. History persists across
      restarts.</p>
      <div class="card">
        <form hx-post="/api/chat" hx-target="#chat-log" hx-swap="afterbegin" hx-indicator="#chat-spinner"
              hx-on::after-request="this.reset()">
          <input type="text" name="question" placeholder="Ask about the docs…" autocomplete="off" required
                 style="width:100%;padding:0.65rem 0.9rem;border:1px solid var(--border);border-radius:8px;font-size:0.95rem;" />
        </form>
        <div id="chat-spinner" class="htmx-indicator muted">thinking…</div>
      </div>
      <div id="chat-log">${historyHtml}</div>
    `;
    res.send(layout({ title: "chat", collections: service.collections(), activeNav: "chat", body }));
  });

  app.get("/chat", (_req, res) => res.redirect("/"));

  app.get("/fragments/system", async (_req, res) => {
    res.send(systemFragment(await service.getOverview()));
  });

  app.get("/api/system", async (_req, res) => {
    res.json(await service.getOverview());
  });

  app.get("/system", async (_req, res) => {
    const collections = service.collections();
    const [s, recentByCollection] = await Promise.all([
      service.getOverview(),
      Promise.all(collections.map((c) => service.listDocs(c))),
    ]);
    const recent = recentByCollection.flat().sort((a, b) => b.modifiedAt.localeCompare(a.modifiedAt)).slice(0, 10);
    const body = `
      <h1>system overview</h1>
      <div hx-get="/fragments/system" hx-trigger="load, every 5s" hx-swap="innerHTML">${systemFragment(s)}</div>
      <h2>Recently updated docs</h2>
      <div class="card">${docListFragment(recent, true)}</div>
    `;
    res.send(layout({ title: "system", collections, activeNav: "system", body }));
  });

  app.post("/api/chat", async (req, res) => {
    const question = String(req.body.question ?? "").trim();
    if (!question) return res.send("");
    const result = await service.askQuestion(question);
    res.send(chatAnswerFragment(result));
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
