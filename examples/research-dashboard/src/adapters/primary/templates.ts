import type { DocEntry, SystemSnapshot } from "../../core/domain/entities.js";

export function fmtBytes(n: number): string {
  if (n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(n) / Math.log(1024));
  return `${(n / 1024 ** i).toFixed(1)} ${units[i]}`;
}

export function layout(opts: {
  title: string;
  collections: string[];
  activeCollection?: string;
  body: string;
}): string {
  const navItems = opts.collections
    .map(
      (c) =>
        `<a href="/docs/${c}" class="${c === opts.activeCollection ? "active" : ""}">${c}</a>`,
    )
    .join("");

  return `<!doctype html><html><head><meta charset="utf-8">
<title>${opts.title} — research-dashboard</title>
<script src="/htmx.min.js"></script>
<style>
  :root { color-scheme: light; }
  body { font-family: -apple-system, sans-serif; margin: 0; display: flex; color: #1a1a1a; }
  aside { width: 200px; flex: none; background: #f7f7f8; padding: 1rem; height: 100vh; box-sizing: border-box; position: sticky; top: 0; }
  aside a { display: block; padding: 0.35rem 0.5rem; border-radius: 4px; color: #333; text-decoration: none; margin-bottom: 0.15rem; }
  aside a:hover, aside a.active { background: #e4e4ff; }
  main { flex: 1; padding: 1.5rem 2rem; max-width: 900px; }
  table { border-collapse: collapse; margin: 1rem 0; }
  td, th { border: 1px solid #ddd; padding: 0.4rem 0.8rem; text-align: left; }
  pre { background: #f5f5f5; padding: 0.8rem; overflow-x: auto; }
  code { background: #f5f5f5; padding: 0.1rem 0.3rem; }
  a { color: #0645ad; }
  #search-box { width: 100%; padding: 0.5rem; font-size: 1rem; box-sizing: border-box; margin-bottom: 1rem; }
  #search-results li, .doc-list li { margin-bottom: 0.3rem; }
  .muted { color: #777; font-size: 0.9em; }
</style></head>
<body>
<aside>
  <a href="/" class="${!opts.activeCollection ? "active" : ""}">home</a>
  <a href="/system">system</a>
  <hr>
  ${navItems}
</aside>
<main>
  <input id="search-box" type="search" placeholder="Search all docs…" autocomplete="off"
    hx-get="/search" hx-trigger="keyup changed delay:250ms, search" hx-target="#search-results"
    name="q" />
  <div id="search-results"></div>
  ${opts.body}
</main>
</body></html>`;
}

export function systemFragment(s: SystemSnapshot): string {
  const gpuRows = s.gpus
    .map(
      (g) =>
        `<tr><td>${g.name}</td><td>${fmtBytes(g.memoryUsedMb * 1024 * 1024)} / ${fmtBytes(g.memoryTotalMb * 1024 * 1024)}</td><td>${g.utilizationPct}%</td></tr>`,
    )
    .join("");
  return `
    <p class="muted">${s.hostname} — as of ${s.takenAt}</p>
    <table>
      <tr><th>CPU</th><td>${s.cpu.model} (${s.cpu.cores} cores), load ${s.cpu.loadAvg1.toFixed(2)}/${s.cpu.loadAvg5.toFixed(2)}/${s.cpu.loadAvg15.toFixed(2)}</td></tr>
      <tr><th>Memory</th><td>${fmtBytes(s.memory.usedBytes)} / ${fmtBytes(s.memory.totalBytes)}</td></tr>
      <tr><th>Disk (${s.disk.mount})</th><td>${fmtBytes(s.disk.usedBytes)} / ${fmtBytes(s.disk.totalBytes)}</td></tr>
    </table>
    ${s.gpus.length ? `<h3>GPU</h3><table><tr><th>Name</th><th>Memory</th><th>Util</th></tr>${gpuRows}</table>` : ""}
  `;
}

export function docListFragment(entries: DocEntry[], showCollection = false): string {
  if (entries.length === 0) return "<p class=\"muted\">nothing here</p>";
  const rows = entries
    .map(
      (e) =>
        `<li><a href="/docs/${e.collection}/${e.relativePath}">${showCollection ? `[${e.collection}] ` : ""}${e.name}</a> <span class="muted">— ${fmtBytes(e.sizeBytes)}, ${e.modifiedAt}</span></li>`,
    )
    .join("");
  return `<ul class="doc-list">${rows}</ul>`;
}
