import type { ChatAnswer, DocEntry, SystemSnapshot } from "../../core/domain/entities.js";

export function fmtBytes(n: number): string {
  if (n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(n) / Math.log(1024));
  return `${(n / 1024 ** i).toFixed(1)} ${units[i]}`;
}

const COLLECTION_ICONS: Record<string, string> = {
  benchmarks: "📊",
  adrs: "📜",
  analysis: "🔍",
  specs: "📋",
  guides: "📘",
  algebra: "🧮",
  reference: "📚",
  examples: "🧩",
  workplans: "🗂️",
};

// PARA (Projects/Areas/Resources/Archives) grouping for the sidebar — presentation-only,
// doesn't touch where files actually live on disk. Order here is the display order.
const PARA_GROUPS: { label: string; collections: string[] }[] = [
  { label: "Projects", collections: ["workplans"] },
  { label: "Areas", collections: ["specs"] },
  { label: "Resources", collections: ["guides", "reference", "algebra", "examples"] },
  { label: "Archives", collections: ["adrs", "analysis", "benchmarks"] },
];

function paraGroupsFor(collections: string[]): { label: string; collections: string[] }[] {
  const known = new Set(PARA_GROUPS.flatMap((g) => g.collections));
  const leftover = collections.filter((c) => !known.has(c));
  const groups = PARA_GROUPS.map((g) => ({ ...g, collections: g.collections.filter((c) => collections.includes(c)) })).filter(
    (g) => g.collections.length > 0,
  );
  return leftover.length > 0 ? [...groups, { label: "Other", collections: leftover }] : groups;
}

const STYLE = `
  :root {
    color-scheme: light;
    --bg: #f4f5f7; --surface: #ffffff; --border: #e3e5ea; --text: #1c1e21; --muted: #6b7280;
    --sidebar-bg: #191b23; --sidebar-text: #c7c9d1; --sidebar-active: #2a2d3a;
    --accent: #6366f1; --accent-hover: #4f46e5; --shadow: 0 1px 3px rgba(0,0,0,0.06), 0 1px 2px rgba(0,0,0,0.04);
  }
  * { box-sizing: border-box; }
  body {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
    margin: 0; display: flex; min-height: 100vh; background: var(--bg); color: var(--text);
    line-height: 1.5;
  }
  aside {
    width: 220px; flex: none; background: var(--sidebar-bg); color: var(--sidebar-text);
    padding: 1.25rem 0.85rem; min-height: 100vh; box-sizing: border-box; position: sticky; top: 0;
  }
  aside .brand { font-weight: 700; color: #fff; font-size: 1.05rem; padding: 0 0.5rem 1rem; letter-spacing: -0.02em; }
  aside .brand span { color: var(--accent); }
  aside a {
    display: flex; align-items: center; gap: 0.55rem; padding: 0.45rem 0.6rem; border-radius: 6px;
    color: var(--sidebar-text); text-decoration: none; margin-bottom: 0.1rem; font-size: 0.92rem;
    transition: background 0.12s ease, color 0.12s ease;
  }
  aside a:hover { background: var(--sidebar-active); color: #fff; }
  aside a.active { background: var(--accent); color: #fff; font-weight: 600; }
  aside hr { border: none; border-top: 1px solid #2c2f3a; margin: 0.75rem 0; }
  aside .section-label { font-size: 0.72rem; text-transform: uppercase; letter-spacing: 0.06em; color: #6b6f80; padding: 0.4rem 0.6rem 0.2rem; margin-top: 0.5rem; }
  aside .section-label:first-of-type { margin-top: 0; }
  main { flex: 1; padding: 2rem 2.5rem; max-width: 960px; }
  h1 { font-size: 1.5rem; margin: 0 0 1rem; letter-spacing: -0.01em; }
  h2 { font-size: 1.1rem; margin: 1.75rem 0 0.75rem; color: var(--text); }
  h3 { font-size: 0.95rem; margin: 1.25rem 0 0.5rem; color: var(--muted); text-transform: uppercase; letter-spacing: 0.04em; }
  .card { background: var(--surface); border: 1px solid var(--border); border-radius: 10px; padding: 1.25rem 1.5rem; box-shadow: var(--shadow); margin-bottom: 1.25rem; }
  .stat-grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 0.9rem; margin: 0.5rem 0 1rem; }
  .stat-card { background: var(--surface); border: 1px solid var(--border); border-radius: 10px; padding: 0.9rem 1.1rem; box-shadow: var(--shadow); }
  .stat-card .label { font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.05em; color: var(--muted); margin-bottom: 0.3rem; }
  .stat-card .value { font-size: 1.05rem; font-weight: 600; }
  .bar { height: 6px; background: #edeef2; border-radius: 3px; margin-top: 0.5rem; overflow: hidden; }
  .bar > div { height: 100%; background: var(--accent); border-radius: 3px; }
  table { border-collapse: collapse; margin: 0.5rem 0; width: 100%; }
  td, th { border-bottom: 1px solid var(--border); padding: 0.5rem 0.7rem; text-align: left; font-size: 0.92rem; }
  th { color: var(--muted); font-weight: 600; font-size: 0.8rem; text-transform: uppercase; letter-spacing: 0.03em; }
  pre { background: #f6f7f9; border: 1px solid var(--border); border-radius: 8px; padding: 1rem; overflow-x: auto; }
  code { background: #f0f1f5; padding: 0.15rem 0.4rem; border-radius: 4px; font-size: 0.9em; }
  pre code { background: none; padding: 0; }
  a { color: var(--accent); text-decoration: none; }
  a:hover { color: var(--accent-hover); text-decoration: underline; }
  #search-box {
    width: 100%; padding: 0.65rem 0.9rem; font-size: 0.95rem; border: 1px solid var(--border);
    border-radius: 8px; margin-bottom: 1.25rem; background: var(--surface); box-shadow: var(--shadow);
  }
  #search-box:focus { outline: none; border-color: var(--accent); box-shadow: 0 0 0 3px rgba(99,102,241,0.15); }
  .doc-list { list-style: none; margin: 0; padding: 0; }
  .doc-list li { padding: 0.65rem 0.25rem; border-bottom: 1px solid var(--border); }
  .doc-list li:last-child { border-bottom: none; }
  .doc-list a { font-weight: 500; }
  .badge { display: inline-block; background: #ecebfe; color: var(--accent-hover); font-size: 0.72rem; font-weight: 600; padding: 0.1rem 0.5rem; border-radius: 999px; margin-right: 0.5rem; }
  .muted { color: var(--muted); font-size: 0.85em; }
  #search-results:not(:empty) { background: var(--surface); border: 1px solid var(--border); border-radius: 10px; box-shadow: var(--shadow); margin: -0.75rem 0 1.25rem; padding: 0.25rem 1rem; }
  .htmx-request.htmx-indicator, .htmx-request .htmx-indicator { opacity: 1; }
  .htmx-indicator { opacity: 0; transition: opacity 0.15s; font-size: 0.8rem; color: var(--muted); }
`;

export function layout(opts: {
  title: string;
  collections: string[];
  activeCollection?: string;
  activeNav?: "chat" | "system";
  body: string;
}): string {
  const navGroups = paraGroupsFor(opts.collections)
    .map(
      (g) => `
        <div class="section-label">${g.label}</div>
        ${g.collections
          .map(
            (c) =>
              `<a href="/docs/${c}" class="${c === opts.activeCollection ? "active" : ""}">${COLLECTION_ICONS[c] ?? "📄"} ${c}</a>`,
          )
          .join("")}`,
    )
    .join("");

  return `<!doctype html><html><head><meta charset="utf-8">
<title>${opts.title} — research-dashboard</title>
<script src="/htmx.min.js"></script>
<style>${STYLE}</style></head>
<body>
<aside>
  <div class="brand">research<span>.</span>dashboard</div>
  <a href="/" class="${opts.activeNav === "chat" ? "active" : ""}">💬 chat</a>
  <a href="/system" class="${opts.activeNav === "system" ? "active" : ""}">🖥️ system</a>
  <hr>
  ${navGroups}
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
  const memPct = Math.round((s.memory.usedBytes / s.memory.totalBytes) * 100);
  const diskPct = Math.round((s.disk.usedBytes / s.disk.totalBytes) * 100);
  const gpuCards = s.gpus
    .map((g) => {
      const gpuPct = Math.round((g.memoryUsedMb / g.memoryTotalMb) * 100);
      return `
        <div class="stat-card">
          <div class="label">${g.name}</div>
          <div class="value">${fmtBytes(g.memoryUsedMb * 1024 * 1024)} / ${fmtBytes(g.memoryTotalMb * 1024 * 1024)}</div>
          <div class="bar"><div style="width:${gpuPct}%"></div></div>
          <div class="muted">${g.utilizationPct}% utilization</div>
        </div>`;
    })
    .join("");

  return `
    <p class="muted">${s.hostname} &middot; updated ${new Date(s.takenAt).toLocaleTimeString()}</p>
    <div class="stat-grid">
      <div class="stat-card">
        <div class="label">CPU</div>
        <div class="value">${s.cpu.cores} cores</div>
        <div class="muted">${s.cpu.model}</div>
        <div class="muted">load ${s.cpu.loadAvg1.toFixed(2)} / ${s.cpu.loadAvg5.toFixed(2)} / ${s.cpu.loadAvg15.toFixed(2)}</div>
      </div>
      <div class="stat-card">
        <div class="label">Memory</div>
        <div class="value">${fmtBytes(s.memory.usedBytes)} / ${fmtBytes(s.memory.totalBytes)}</div>
        <div class="bar"><div style="width:${memPct}%"></div></div>
      </div>
      <div class="stat-card">
        <div class="label">Disk (${s.disk.mount})</div>
        <div class="value">${fmtBytes(s.disk.usedBytes)} / ${fmtBytes(s.disk.totalBytes)}</div>
        <div class="bar"><div style="width:${diskPct}%"></div></div>
      </div>
      ${gpuCards}
    </div>
  `;
}

export function docListFragment(entries: DocEntry[], showCollection = false): string {
  if (entries.length === 0) return "<p class=\"muted\">nothing here</p>";
  const rows = entries
    .map(
      (e) =>
        `<li>${showCollection ? `<span class="badge">${e.collection}</span>` : ""}<a href="/docs/${e.collection}/${e.relativePath}">${e.name}</a> <span class="muted">— ${fmtBytes(e.sizeBytes)}, ${new Date(e.modifiedAt).toLocaleDateString()}</span></li>`,
    )
    .join("");
  return `<ul class="doc-list">${rows}</ul>`;
}

export function chatAnswerFragment(result: ChatAnswer): string {
  const sources = result.sources
    .map(
      (s) =>
        `<li><a href="/docs/${s.collection}/${s.relativePath}">[${s.collection}] ${s.name}</a> <span class="muted">(${s.score.toFixed(2)})</span></li>`,
    )
    .join("");
  return `
    <div class="card">
      <p class="muted">${new Date(result.answeredAt).toLocaleString()}</p>
      <p><strong>Q:</strong> ${result.question}</p>
      <p>${result.answer.replace(/\n/g, "<br>")}</p>
      ${result.sources.length ? `<h3>sources</h3><ul class="doc-list">${sources}</ul>` : ""}
    </div>
  `;
}
