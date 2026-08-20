/**
 * KnowledgeGraphView — dashboard surface for the hex-graph engine.
 *
 * Exposes the /api/graph/* endpoints: build/summary (stats), query (search),
 * explain (node + neighbourhood), and path (shortest path). Rendering the full
 * 17k-node graph isn't practical, so this is an *explorer*: search → select →
 * see a bounded neighbourhood (SVG radial) + the node's relationships.
 */
import { Component, createSignal, For, Show } from 'solid-js';
import { restClient } from '../../services/rest-client';

const DEFAULT_PATH = '.';

const KIND_COLOR: Record<string, string> = {
  file: '#60a5fa',
  function: '#34d399',
  struct: '#f472b6',
  class: '#f472b6',
  interface: '#c084fc',
  type: '#fbbf24',
  enum: '#fb923c',
  const: '#94a3b8',
  trait: '#22d3ee',
  doc_concept: '#a3e635',
};
const kindColor = (k?: string) => (k && KIND_COLOR[k]) || '#9ca3af';

interface Ranked { id: string; label: string; kind: string; file: string; score: number }
interface Neighbor { label: string; relation: string; confidence: string; direction: string }
interface Explanation {
  id: string; label: string; kind: string; file: string; line: number;
  degree: number; community: number; community_label: string; neighbors: Neighbor[];
}

const KnowledgeGraphView: Component = () => {
  const [path, setPath] = createSignal(DEFAULT_PATH);

  const [summary, setSummary] = createSignal<any | null>(null);
  const [summaryErr, setSummaryErr] = createSignal('');
  const [building, setBuilding] = createSignal(false);

  const loadSummary = async () => {
    setSummaryErr('');
    try {
      setSummary(await restClient.post('/api/graph/summary', { path: path() }));
    } catch (e: any) {
      setSummary(null);
      setSummaryErr(e?.message ?? String(e));
    }
  };
  // Initial load (fire-and-forget).
  void loadSummary();

  const build = async (mode: 'ast' | 'deep') => {
    setBuilding(true);
    setSummaryErr('');
    try {
      await restClient.post('/api/graph/build', { path: path(), mode, persist: false });
      await loadSummary();
    } catch (e: any) {
      setSummaryErr(e?.message ?? String(e));
    } finally {
      setBuilding(false);
    }
  };

  // ── search / query ──
  const [q, setQ] = createSignal('');
  const [results, setResults] = createSignal<Ranked[]>([]);
  const [querying, setQuerying] = createSignal(false);
  const runQuery = async () => {
    if (!q().trim()) return;
    setQuerying(true);
    try {
      const r = await restClient.post<{ results: Ranked[] }>('/api/graph/query', {
        path: path(), question: q(), limit: 25,
      });
      setResults(r.results ?? []);
    } catch {
      setResults([]);
    } finally {
      setQuerying(false);
    }
  };

  // ── explain / focus ──
  const [focus, setFocus] = createSignal<Explanation | null>(null);
  const explain = async (node: string) => {
    try {
      setFocus(await restClient.post<Explanation>('/api/graph/explain', { path: path(), node }));
    } catch {
      setFocus(null);
    }
  };

  // ── path ──
  const [pf, setPf] = createSignal('');
  const [pt, setPt] = createSignal('');
  const [pathRes, setPathRes] = createSignal<{ found: boolean; labels?: string[] } | null>(null);
  const runPath = async () => {
    if (!pf().trim() || !pt().trim()) return;
    try {
      setPathRes(await restClient.post('/api/graph/path', { path: path(), from: pf(), to: pt() }));
    } catch {
      setPathRes({ found: false });
    }
  };

  // Bounded neighbourhood positions for the SVG radial.
  const neighborNodes = () => {
    const f = focus();
    if (!f) return [];
    const ns = f.neighbors.slice(0, 24);
    const cx = 200, cy = 200, r = 150;
    return ns.map((n, i) => {
      const a = (2 * Math.PI * i) / Math.max(ns.length, 1) - Math.PI / 2;
      return { ...n, x: cx + r * Math.cos(a), y: cy + r * Math.sin(a) };
    });
  };

  return (
    <div class="flex flex-1 flex-col overflow-auto p-4 gap-4 text-sm text-zinc-200">
      {/* Header */}
      <div class="flex items-center gap-3 flex-wrap">
        <h1 class="text-lg font-semibold text-zinc-100">Knowledge Graph</h1>
        <input
          class="rounded border border-zinc-700 bg-zinc-900 px-2 py-1 text-xs w-64"
          value={path()}
          onInput={(e) => setPath(e.currentTarget.value)}
          placeholder="project path (default '.')"
        />
        <button
          class="rounded bg-blue-600 hover:bg-blue-500 px-3 py-1 text-xs font-medium disabled:opacity-50"
          disabled={building()}
          onClick={() => build('ast')}
        >
          {building() ? 'Building…' : 'Build / Rebuild (AST)'}
        </button>
        <button
          class="rounded border border-zinc-600 hover:bg-zinc-800 px-3 py-1 text-xs disabled:opacity-50"
          disabled={building()}
          onClick={() => build('deep')}
          title="AST + LLM-inferred edges from docs (requires an inference provider)"
        >
          Deep
        </button>
      </div>

      {/* No graph yet / error */}
      <Show when={summaryErr()}>
        <div class="rounded border border-amber-700/50 bg-amber-950/30 p-3 text-amber-300 text-xs">
          {summaryErr()} — click <b>Build</b> to generate the graph for this path.
        </div>
      </Show>

      {/* Stats */}
      <Show when={summary()}>
        {(s) => (
          <div class="grid grid-cols-2 md:grid-cols-4 gap-3">
            <Stat label="Nodes" value={s().node_count} />
            <Stat label="Edges" value={s().edge_count} />
            <Stat label="Communities" value={s().community_count} />
            <Stat label="Mode" value={s().mode} />
            <div class="col-span-2 md:col-span-4 rounded border border-zinc-800 bg-zinc-900/50 p-3">
              <div class="text-xs uppercase tracking-wide text-zinc-500 mb-1">Hub nodes (highest degree)</div>
              <div class="flex flex-wrap gap-1">
                <For each={s().god_nodes ?? []}>
                  {(g: string) => (
                    <button
                      class="rounded bg-zinc-800 hover:bg-zinc-700 px-2 py-0.5 text-xs"
                      onClick={() => explain(g)}
                    >{g}</button>
                  )}
                </For>
              </div>
            </div>
          </div>
        )}
      </Show>

      <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
        {/* Search + results */}
        <div class="rounded border border-zinc-800 bg-zinc-900/50 p-3 flex flex-col gap-2">
          <div class="text-xs uppercase tracking-wide text-zinc-500">Search</div>
          <div class="flex gap-2">
            <input
              class="flex-1 rounded border border-zinc-700 bg-zinc-900 px-2 py-1 text-xs"
              value={q()}
              onInput={(e) => setQ(e.currentTarget.value)}
              onKeyDown={(e) => e.key === 'Enter' && runQuery()}
              placeholder="e.g. inference routing, KnowledgeGraph"
            />
            <button class="rounded bg-blue-600 hover:bg-blue-500 px-3 py-1 text-xs" onClick={runQuery}>
              {querying() ? '…' : 'Search'}
            </button>
          </div>
          <div class="flex flex-col gap-1 overflow-auto max-h-80">
            <For each={results()}>
              {(r) => (
                <button
                  class="text-left rounded hover:bg-zinc-800 px-2 py-1 flex items-center gap-2"
                  onClick={() => explain(r.id)}
                >
                  <span class="inline-block w-2 h-2 rounded-full" style={{ background: kindColor(r.kind) }} />
                  <span class="font-medium">{r.label}</span>
                  <span class="text-zinc-500 text-xs">{r.kind}</span>
                  <span class="text-zinc-600 text-xs truncate ml-auto">{r.file}</span>
                </button>
              )}
            </For>
            <Show when={!querying() && results().length === 0}>
              <span class="text-zinc-600 text-xs px-2">No results yet — search or click a hub node.</span>
            </Show>
          </div>

          {/* Path tool */}
          <div class="text-xs uppercase tracking-wide text-zinc-500 mt-2">Shortest path</div>
          <div class="flex gap-2 items-center flex-wrap">
            <input class="rounded border border-zinc-700 bg-zinc-900 px-2 py-1 text-xs w-40"
              value={pf()} onInput={(e) => setPf(e.currentTarget.value)} placeholder="from (label or id)" />
            <span class="text-zinc-500">→</span>
            <input class="rounded border border-zinc-700 bg-zinc-900 px-2 py-1 text-xs w-40"
              value={pt()} onInput={(e) => setPt(e.currentTarget.value)} placeholder="to (label or id)" />
            <button class="rounded border border-zinc-600 hover:bg-zinc-800 px-3 py-1 text-xs" onClick={runPath}>Find</button>
          </div>
          <Show when={pathRes()}>
            {(p) => (
              <div class="text-xs text-zinc-300">
                <Show when={p().found} fallback={<span class="text-zinc-500">No path found.</span>}>
                  {(p() as any).labels?.join('  →  ')}
                </Show>
              </div>
            )}
          </Show>
        </div>

        {/* Focus / neighbourhood */}
        <div class="rounded border border-zinc-800 bg-zinc-900/50 p-3 flex flex-col gap-2">
          <div class="text-xs uppercase tracking-wide text-zinc-500">Node detail</div>
          <Show when={focus()} fallback={<span class="text-zinc-600 text-xs">Select a node to inspect its neighbourhood.</span>}>
            {(f) => (
              <>
                <div class="flex items-center gap-2">
                  <span class="inline-block w-3 h-3 rounded-full" style={{ background: kindColor(f().kind) }} />
                  <span class="font-semibold text-zinc-100">{f().label}</span>
                  <span class="text-zinc-500 text-xs">{f().kind}</span>
                </div>
                <div class="text-xs text-zinc-400">
                  {f().file}{f().line ? `:${f().line}` : ''} · degree {f().degree} · community “{f().community_label}”
                </div>

                {/* SVG radial neighbourhood */}
                <svg viewBox="0 0 400 400" class="w-full max-h-96 my-1">
                  <For each={neighborNodes()}>
                    {(n: any) => (
                      <line x1="200" y1="200" x2={n.x} y2={n.y}
                        stroke={n.direction === 'out' ? '#3b82f6' : '#22c55e'}
                        stroke-opacity={n.confidence === 'extracted' ? '0.7' : '0.3'} stroke-width="1" />
                    )}
                  </For>
                  <For each={neighborNodes()}>
                    {(n: any) => (
                      <g>
                        <circle cx={n.x} cy={n.y} r="6" fill="#27272a" stroke={n.direction === 'out' ? '#3b82f6' : '#22c55e'} stroke-width="2"
                          style={{ cursor: 'pointer' }} onClick={() => explain(n.label)} />
                        <text x={n.x} y={n.y - 9} font-size="9" fill="#a1a1aa" text-anchor="middle">
                          {n.label.length > 16 ? n.label.slice(0, 15) + '…' : n.label}
                        </text>
                      </g>
                    )}
                  </For>
                  <circle cx="200" cy="200" r="11" fill={kindColor(f().kind)} />
                  <text x="200" y="225" font-size="11" fill="#e4e4e7" text-anchor="middle" font-weight="600">
                    {f().label.length > 22 ? f().label.slice(0, 21) + '…' : f().label}
                  </text>
                </svg>

                {/* Relationship list */}
                <div class="flex flex-col gap-0.5 overflow-auto max-h-48 text-xs">
                  <For each={f().neighbors}>
                    {(n) => (
                      <div class="flex items-center gap-2 px-1">
                        <span class="text-zinc-500 w-4">{n.direction === 'out' ? '→' : '←'}</span>
                        <span class="text-zinc-400 w-24">{n.relation}</span>
                        <button class="text-zinc-200 hover:underline truncate" onClick={() => explain(n.label)}>{n.label}</button>
                        <span class="ml-auto text-[10px] text-zinc-600">[{n.confidence}]</span>
                      </div>
                    )}
                  </For>
                </div>
              </>
            )}
          </Show>
        </div>
      </div>
    </div>
  );
};

const Stat: Component<{ label: string; value: any }> = (props) => (
  <div class="rounded border border-zinc-800 bg-zinc-900/50 p-3">
    <div class="text-2xl font-semibold text-zinc-100">{props.value ?? '—'}</div>
    <div class="text-xs uppercase tracking-wide text-zinc-500">{props.label}</div>
  </div>
);

export default KnowledgeGraphView;
