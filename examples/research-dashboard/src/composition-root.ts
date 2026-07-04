import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createHttpApp } from "./adapters/primary/http-adapter.js";
import { DocsFsReader } from "./adapters/secondary/docs-fs.js";
import { SystemStatsReader } from "./adapters/secondary/system-stats.js";
import { DashboardService } from "./core/usecases/service.js";

const here = dirname(fileURLToPath(import.meta.url));
// examples/research-dashboard/src -> up 3 levels -> the hex repo root -> docs/
const hexRepoRoot = resolve(here, "..", "..", "..");

const COLLECTIONS = ["benchmarks", "adrs", "analysis", "specs", "guides", "algebra", "reference", "examples"];
const docs = new DocsFsReader(
  Object.fromEntries(COLLECTIONS.map((c) => [c, resolve(hexRepoRoot, "docs", c)])),
);
const stats = new SystemStatsReader("/");
const service = new DashboardService(stats, docs);
const app = createHttpApp(service);

const port = Number(process.env.PORT ?? 8090);
const host = process.env.HOST ?? "0.0.0.0";
app.listen(port, host, () => {
  console.log(`research-dashboard listening on http://${host}:${port}`);
});
