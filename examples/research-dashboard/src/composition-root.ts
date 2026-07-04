import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createHttpApp } from "./adapters/primary/http-adapter.js";
import { DocsFsReader } from "./adapters/secondary/docs-fs.js";
import { EmbeddingKnowledgeIndex } from "./adapters/secondary/embedding-knowledge-index.js";
import { OllamaChatModel } from "./adapters/secondary/ollama-chat-model.js";
import { OllamaEmbedder } from "./adapters/secondary/ollama-embedder.js";
import { SqliteChatHistoryStore } from "./adapters/secondary/sqlite-chat-history-store.js";
import { SystemStatsReader } from "./adapters/secondary/system-stats.js";
import { DashboardService } from "./core/usecases/service.js";

const here = dirname(fileURLToPath(import.meta.url));
// examples/research-dashboard/src -> up 3 levels -> the hex repo root -> docs/
const hexRepoRoot = resolve(here, "..", "..", "..");

const COLLECTIONS = ["benchmarks", "adrs", "analysis", "specs", "guides", "algebra", "reference", "examples", "workplans"];
const docs = new DocsFsReader(
  Object.fromEntries(COLLECTIONS.map((c) => [c, resolve(hexRepoRoot, "docs", c)])),
);
const stats = new SystemStatsReader("/");

const ollamaUrl = process.env.OLLAMA_URL ?? "http://localhost:11434";
const embedder = new OllamaEmbedder(ollamaUrl, process.env.OLLAMA_EMBED_MODEL ?? "nomic-embed-text");
const chatModel = new OllamaChatModel(ollamaUrl, process.env.OLLAMA_CHAT_MODEL ?? "devstral-small-2:24b");
const knowledge = new EmbeddingKnowledgeIndex(embedder, docs);
knowledge.ensureIndexed(); // kicks off in the background; first /chat request awaits it if not done yet
const history = new SqliteChatHistoryStore(resolve(here, "..", ".data", "chat-history.db"));

const service = new DashboardService(stats, docs, knowledge, chatModel, history);
const app = createHttpApp(service);

const port = Number(process.env.PORT ?? 8090);
const host = process.env.HOST ?? "0.0.0.0";
app.listen(port, host, () => {
  console.log(`research-dashboard listening on http://${host}:${port}`);
});
