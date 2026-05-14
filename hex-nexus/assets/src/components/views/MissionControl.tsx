/**
 * MissionControl.tsx — operator's agent ops console (Hermes-shaped).
 *
 * Layout:
 *   left rail (280px):  active runs · recent runs · STDB chips · drill-downs
 *   main column:        transcript of selected run — LLM reasoning, tool_use
 *                       blocks, tool_results, finish summary
 *   sticky bottom:      compose box dispatching POST /api/agent/run
 *
 * Runs are held in browser state (last 20). Future: persisted via a new
 * `agent_run` STDB table + SSE stream from /api/agent/run.
 */

import { Component, For, Show, createSignal, onMount, onCleanup, createMemo } from "solid-js";
import { restClient } from "../../services/rest-client";
import { navigate } from "../../stores/router";

interface AgentStep {
  iteration: number;
  tool: string;
  input: any;
  ok: boolean;
  output: any;
  error: string | null;
  elapsed_ms: number;
  assistant_text?: string;
}

interface RunResult {
  iterations: number;
  steps: AgentStep[];
  final_text: string;
  stop_reason: string;
  elapsed_ms: number;
}

interface RunRecord {
  id: string;           // local-only id (uuid-ish from Date.now + random)
  intent: string;
  model: string;
  max_iterations: number;
  started_at: number;   // Date.now()
  status: "running" | "done" | "error";
  result?: RunResult;
  error?: string;
}

interface StatsPayload {
  stdb_alive: boolean;
  autonomous_commits_today: number;
  p0_count: number;
  p1_count: number;
}

const REFRESH_MS = 5000;
const HISTORY_KEY = "hex.mission_control.run_history";
const HISTORY_LIMIT = 20;
const MODELS = [
  { id: "anthropic/claude-haiku-4.5", label: "haiku-4.5" },
  { id: "anthropic/claude-sonnet-4-6", label: "sonnet-4.6" },
  { id: "anthropic/claude-opus-4-7", label: "opus-4.7" },
];

const formatAge = (ms: number): string => {
  const s = Math.max(0, Math.floor((Date.now() - ms) / 1000));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
};

const formatDuration = (ms: number): string => {
  if (ms < 1000) return `${ms}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}s`;
  return `${Math.floor(s / 60)}m ${Math.floor(s % 60)}s`;
};

const truncate = (s: string, n: number): string =>
  s.length > n ? s.slice(0, n) + "…" : s;

const stopReasonColor = (r: string): string => {
  if (r === "finished" || r === "finished_after_duplicate_success") return "text-green-400";
  if (r === "max_iterations") return "text-amber-400";
  if (r === "no_tool_use") return "text-blue-400";
  return "text-red-400";
};

const toolIcon = (tool: string): string => {
  switch (tool) {
    case "code_patch": return "✎";
    case "cargo_check": case "typescript_check": return "✓";
    case "repo_grep": case "repo_read": return "?";
    case "adr_draft": case "spec_draft": case "workplan_emit": return "📝";
    case "finish": return "▣";
    case "escalate_to_operator": return "⚠";
    default: return "·";
  }
};

const loadHistory = (): RunRecord[] => {
  try {
    const raw = localStorage.getItem(HISTORY_KEY);
    if (!raw) return [];
    const arr = JSON.parse(raw);
    return Array.isArray(arr) ? arr : [];
  } catch {
    return [];
  }
};

const saveHistory = (runs: RunRecord[]) => {
  try {
    localStorage.setItem(HISTORY_KEY, JSON.stringify(runs.slice(0, HISTORY_LIMIT)));
  } catch {}
};

const MissionControl: Component = () => {
  const [history, setHistory] = createSignal<RunRecord[]>(loadHistory());
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [intent, setIntent] = createSignal("");
  const [model, setModel] = createSignal(MODELS[0].id);
  const [maxIter, setMaxIter] = createSignal(6);
  const [running, setRunning] = createSignal(false);
  const [stats, setStats] = createSignal<StatsPayload | null>(null);
  const [expanded, setExpanded] = createSignal<Set<string>>(new Set());

  let statsTimer: ReturnType<typeof setInterval> | null = null;

  const refreshStats = async () => {
    try {
      const d = await restClient.get("/api/mission-control");
      const af = d.attention_feed || [];
      setStats({
        stdb_alive: !!d.stdb_alive,
        autonomous_commits_today: d.pulse?.autonomous_commits_today ?? 0,
        p0_count: af.filter((i: any) => i.priority === 0).length,
        p1_count: af.filter((i: any) => i.priority === 1).length,
      });
    } catch {
      setStats({ stdb_alive: false, autonomous_commits_today: 0, p0_count: 0, p1_count: 0 });
    }
  };

  onMount(() => {
    refreshStats();
    statsTimer = setInterval(refreshStats, REFRESH_MS);
    const h = loadHistory();
    if (h.length > 0 && !selectedId()) setSelectedId(h[0].id);
  });
  onCleanup(() => { if (statsTimer) clearInterval(statsTimer); });

  const persist = (runs: RunRecord[]) => {
    setHistory(runs);
    saveHistory(runs);
  };

  const dispatch = async () => {
    const text = intent().trim();
    if (!text || running()) return;
    setRunning(true);
    const id = `${Date.now()}-${Math.random().toString(36).slice(2, 8)}`;
    const record: RunRecord = {
      id,
      intent: text,
      model: model(),
      max_iterations: maxIter(),
      started_at: Date.now(),
      status: "running",
    };
    persist([record, ...history()]);
    setSelectedId(id);
    setIntent("");
    try {
      const resp = await restClient.post("/api/agent/run", {
        intent: text,
        max_iterations: maxIter(),
        model: model(),
      });
      const updated = history().map((r) =>
        r.id === id ? { ...r, status: "done" as const, result: resp } : r
      );
      persist(updated);
    } catch (e: any) {
      const updated = history().map((r) =>
        r.id === id ? { ...r, status: "error" as const, error: e?.message || String(e) } : r
      );
      persist(updated);
    } finally {
      setRunning(false);
    }
  };

  const selected = createMemo(() => history().find((r) => r.id === selectedId()) || null);

  const clearHistory = () => {
    if (!confirm("Clear all run history?")) return;
    persist([]);
    setSelectedId(null);
  };

  const toggleExpanded = (key: string) => {
    const s = new Set(expanded());
    if (s.has(key)) s.delete(key); else s.add(key);
    setExpanded(s);
  };

  const drillDowns = [
    { label: "Resources", page: "resources" },
    { label: "Personas", page: "persona-health" },
    { label: "Commitments", page: "commitments" },
    { label: "Merge Gate", page: "merge-gate" },
    { label: "Brain", page: "brain" },
    { label: "Org Chart", page: "org-chart" },
    { label: "Thoughts", page: "thoughts" },
    { label: "Missions", page: "missions" },
  ];

  return (
    <div class="flex h-screen bg-zinc-950 text-zinc-100 font-sans">
      {/* ───────────── Left rail ───────────── */}
      <aside class="w-72 shrink-0 flex flex-col border-r border-zinc-800 bg-zinc-950">
        <div class="px-4 py-3 border-b border-zinc-800 flex items-center justify-between">
          <h1 class="text-sm font-semibold tracking-wide">Mission Control</h1>
          <button
            class="text-[10px] text-zinc-500 hover:text-zinc-300"
            onClick={clearHistory}
            title="Clear history"
          >
            clear
          </button>
        </div>
        <div class="flex-1 overflow-y-auto">
          <Show when={history().some((r) => r.status === "running")}>
            <div class="px-4 py-2 text-[10px] uppercase tracking-wide text-zinc-500">Active</div>
            <For each={history().filter((r) => r.status === "running")}>{(r) => (
              <RunListItem run={r} selected={selectedId() === r.id} onSelect={() => setSelectedId(r.id)} />
            )}</For>
          </Show>
          <Show when={history().some((r) => r.status !== "running")}>
            <div class="px-4 py-2 text-[10px] uppercase tracking-wide text-zinc-500 mt-2">Recent</div>
            <For each={history().filter((r) => r.status !== "running")}>{(r) => (
              <RunListItem run={r} selected={selectedId() === r.id} onSelect={() => setSelectedId(r.id)} />
            )}</For>
          </Show>
          <Show when={history().length === 0}>
            <div class="px-4 py-6 text-xs text-zinc-500 italic">
              No runs yet. Type an intent below.
            </div>
          </Show>
        </div>
        {/* footer chips */}
        <div class="px-4 py-3 border-t border-zinc-800 space-y-1.5 text-[11px]">
          <div class="flex items-center justify-between">
            <span class="text-zinc-500">STDB</span>
            <span class={stats()?.stdb_alive ? "text-green-400" : "text-red-400"}>
              {stats()?.stdb_alive ? "✓ up" : "✗ down"}
            </span>
          </div>
          <div class="flex items-center justify-between">
            <span class="text-zinc-500">commits today</span>
            <span class="text-cyan-300 tabular-nums">{stats()?.autonomous_commits_today ?? "—"}</span>
          </div>
          <div class="flex items-center justify-between">
            <span class="text-zinc-500">attention</span>
            <span class="space-x-1.5 tabular-nums">
              <span class={(stats()?.p0_count ?? 0) > 0 ? "text-red-400" : "text-zinc-600"}>{stats()?.p0_count ?? 0} P0</span>
              <span class={(stats()?.p1_count ?? 0) > 0 ? "text-amber-400" : "text-zinc-600"}>{stats()?.p1_count ?? 0} P1</span>
            </span>
          </div>
          <div class="pt-2 mt-2 border-t border-zinc-800 flex flex-wrap gap-x-2 gap-y-1">
            <For each={drillDowns}>{(d) => (
              <button
                class="text-zinc-500 hover:text-cyan-400 text-[10px]"
                onClick={() => navigate({ page: d.page } as any)}
              >
                {d.label}
              </button>
            )}</For>
          </div>
        </div>
      </aside>

      {/* ───────────── Main column ───────────── */}
      <main class="flex-1 flex flex-col min-w-0">
        <Show
          when={selected()}
          fallback={
            <div class="flex-1 flex items-center justify-center text-zinc-500">
              <div class="text-center max-w-md px-6">
                <div class="text-zinc-400 mb-2">No run selected</div>
                <div class="text-xs">
                  Type a natural-language intent below and press <kbd class="px-1.5 py-0.5 bg-zinc-800 rounded text-zinc-300">⌘↵</kbd> to dispatch.
                </div>
              </div>
            </div>
          }
        >
          <Transcript
            run={selected()!}
            expanded={expanded()}
            toggleExpanded={toggleExpanded}
          />
        </Show>

        {/* ───────────── Sticky compose ───────────── */}
        <div class="border-t border-zinc-800 bg-zinc-900/60 px-4 py-3">
          <div class="flex items-center gap-2 mb-2 text-[11px] text-zinc-500">
            <span class="font-mono text-cyan-400">hex agent run</span>
            <select
              class="bg-zinc-900 border border-zinc-700 rounded px-2 py-0.5 text-zinc-300 text-[11px]"
              value={model()}
              onChange={(e) => setModel(e.currentTarget.value)}
              disabled={running()}
            >
              <For each={MODELS}>{(m) => <option value={m.id}>{m.label}</option>}</For>
            </select>
            <label class="flex items-center gap-1">
              <span>iter</span>
              <input
                type="number"
                min="1"
                max="20"
                class="w-12 bg-zinc-900 border border-zinc-700 rounded px-1 py-0.5 text-zinc-300 text-[11px] tabular-nums"
                value={maxIter()}
                onInput={(e) => setMaxIter(Math.max(1, Math.min(20, parseInt(e.currentTarget.value) || 6)))}
                disabled={running()}
              />
            </label>
            <span class="ml-auto text-zinc-600">⌘↵ dispatches</span>
          </div>
          <div class="flex gap-2">
            <textarea
              class="flex-1 bg-zinc-950 border border-zinc-700 focus:border-cyan-600 rounded px-3 py-2 text-sm font-mono resize-none focus:outline-none"
              rows={3}
              placeholder='e.g. "Use code_patch to create docs/specs/foo.md with new_content ..."'
              value={intent()}
              onInput={(e) => setIntent(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                  e.preventDefault();
                  dispatch();
                }
              }}
              disabled={running()}
            />
            <button
              class="px-5 rounded bg-cyan-700 hover:bg-cyan-600 text-white text-sm disabled:opacity-50 disabled:cursor-not-allowed"
              disabled={!intent().trim() || running()}
              onClick={dispatch}
            >
              {running() ? "Running…" : "Run"}
            </button>
          </div>
        </div>
      </main>
    </div>
  );
};

// ─────────────────────── Sub-components ───────────────────────

const RunListItem: Component<{ run: RunRecord; selected: boolean; onSelect: () => void }> = (props) => {
  const statusGlyph = () => {
    if (props.run.status === "running") return <span class="text-cyan-400 animate-pulse">●</span>;
    if (props.run.status === "error") return <span class="text-red-400">✗</span>;
    const sr = props.run.result?.stop_reason || "";
    if (sr === "finished" || sr === "finished_after_duplicate_success") return <span class="text-green-400">✓</span>;
    if (sr === "max_iterations") return <span class="text-amber-400">⚠</span>;
    return <span class="text-zinc-500">·</span>;
  };
  return (
    <button
      class="w-full text-left px-4 py-2 border-l-2 transition-colors hover:bg-zinc-900"
      classList={{
        "border-cyan-500 bg-zinc-900": props.selected,
        "border-transparent": !props.selected,
      }}
      onClick={props.onSelect}
    >
      <div class="flex items-center gap-2 text-[11px]">
        {statusGlyph()}
        <span class="font-mono text-zinc-500">{props.run.id.slice(0, 8)}</span>
        <span class="ml-auto text-zinc-600">{formatAge(props.run.started_at)} ago</span>
      </div>
      <div class="text-xs text-zinc-300 line-clamp-2 mt-0.5">{props.run.intent}</div>
    </button>
  );
};

const Transcript: Component<{
  run: RunRecord;
  expanded: Set<string>;
  toggleExpanded: (k: string) => void;
}> = (props) => {
  const groupedSteps = createMemo(() => {
    if (!props.run.result) return [];
    const groups: { iteration: number; assistant_text: string; steps: AgentStep[] }[] = [];
    for (const step of props.run.result.steps) {
      let g = groups.find((x) => x.iteration === step.iteration);
      if (!g) {
        g = { iteration: step.iteration, assistant_text: step.assistant_text || "", steps: [] };
        groups.push(g);
      }
      // first non-empty assistant_text per iteration wins
      if (!g.assistant_text && step.assistant_text) g.assistant_text = step.assistant_text;
      g.steps.push(step);
    }
    return groups;
  });

  return (
    <div class="flex-1 overflow-y-auto">
      {/* Run header */}
      <div class="px-6 py-4 border-b border-zinc-800 sticky top-0 bg-zinc-950 z-10">
        <div class="flex items-center gap-3 text-[11px] text-zinc-500 mb-1">
          <span class="font-mono">{props.run.id.slice(0, 8)}</span>
          <span class="text-zinc-600">·</span>
          <span>{props.run.model.replace("anthropic/", "")}</span>
          <span class="text-zinc-600">·</span>
          <span>{formatAge(props.run.started_at)} ago</span>
          <Show when={props.run.result}>
            <span class="text-zinc-600">·</span>
            <span>{props.run.result!.iterations} iter · {props.run.result!.steps.length} steps</span>
            <span class="text-zinc-600">·</span>
            <span>{formatDuration(props.run.result!.elapsed_ms)}</span>
            <span class="text-zinc-600">·</span>
            <span class={stopReasonColor(props.run.result!.stop_reason)}>{props.run.result!.stop_reason}</span>
          </Show>
          <Show when={props.run.status === "running"}>
            <span class="text-cyan-400 animate-pulse">● running</span>
          </Show>
          <Show when={props.run.status === "error"}>
            <span class="text-red-400">✗ error</span>
          </Show>
        </div>
        <h2 class="text-sm text-zinc-100 font-mono leading-relaxed">{props.run.intent}</h2>
      </div>

      {/* Body */}
      <div class="px-6 py-4 space-y-4 max-w-4xl">
        <Show when={props.run.status === "running"}>
          <div class="text-xs text-zinc-500 italic">Awaiting response from the inference path…</div>
        </Show>
        <Show when={props.run.error}>
          <div class="rounded border border-red-900 bg-red-950/40 px-3 py-2 text-sm text-red-300">
            {props.run.error}
          </div>
        </Show>
        <For each={groupedSteps()}>
          {(g) => (
            <div class="space-y-2">
              <Show when={g.assistant_text}>
                <div class="rounded border border-zinc-800 bg-zinc-900/40 px-3 py-2 text-sm text-zinc-300 whitespace-pre-wrap">
                  <span class="text-[10px] uppercase tracking-wide text-zinc-500 block mb-1">
                    iter {g.iteration} · assistant
                  </span>
                  {g.assistant_text}
                </div>
              </Show>
              <For each={g.steps}>
                {(step) => {
                  const key = `${props.run.id}-${step.iteration}-${step.tool}`;
                  const isOpen = () => props.expanded.has(key);
                  return (
                    <div
                      class="rounded border bg-zinc-900/60"
                      classList={{
                        "border-green-800": step.ok && !(step.output?.skipped),
                        "border-zinc-700": step.output?.skipped,
                        "border-red-800": !step.ok,
                      }}
                    >
                      <button
                        class="w-full text-left px-3 py-2 flex items-center gap-2 text-xs hover:bg-zinc-900"
                        onClick={() => props.toggleExpanded(key)}
                      >
                        <span class="text-zinc-300 w-4">{toolIcon(step.tool)}</span>
                        <span class="font-mono text-cyan-300">{step.tool}</span>
                        <Show when={step.input?.path}>
                          <span class="text-zinc-500 truncate">{step.input.path}</span>
                        </Show>
                        <span class="ml-auto text-[10px] text-zinc-500">
                          {step.elapsed_ms}ms
                        </span>
                        <Show when={step.output?.skipped}>
                          <span class="text-[10px] text-zinc-500 italic">deduped</span>
                        </Show>
                        <Show when={!step.ok}>
                          <span class="text-[10px] text-red-400">failed</span>
                        </Show>
                        <span class="text-zinc-600 ml-1">{isOpen() ? "▾" : "▸"}</span>
                      </button>
                      <Show when={isOpen()}>
                        <div class="border-t border-zinc-800 px-3 py-2 space-y-2 text-xs">
                          <div>
                            <span class="text-[10px] uppercase tracking-wide text-zinc-500 block mb-1">input</span>
                            <pre class="text-zinc-300 bg-zinc-950 rounded p-2 overflow-x-auto whitespace-pre-wrap break-words">{JSON.stringify(step.input, null, 2)}</pre>
                          </div>
                          <Show when={step.ok}>
                            <div>
                              <span class="text-[10px] uppercase tracking-wide text-zinc-500 block mb-1">output</span>
                              <pre class="text-zinc-300 bg-zinc-950 rounded p-2 overflow-x-auto whitespace-pre-wrap break-words">{truncate(JSON.stringify(step.output, null, 2), 4000)}</pre>
                            </div>
                          </Show>
                          <Show when={step.error}>
                            <div>
                              <span class="text-[10px] uppercase tracking-wide text-red-400 block mb-1">error</span>
                              <pre class="text-red-300 bg-red-950/40 rounded p-2 whitespace-pre-wrap break-words">{step.error}</pre>
                            </div>
                          </Show>
                        </div>
                      </Show>
                    </div>
                  );
                }}
              </For>
            </div>
          )}
        </For>
        <Show when={props.run.result?.final_text}>
          <div class="rounded border border-zinc-700 bg-zinc-900/40 px-3 py-2 text-sm text-zinc-200 whitespace-pre-wrap">
            <span class="text-[10px] uppercase tracking-wide text-zinc-500 block mb-1">final</span>
            {props.run.result!.final_text}
          </div>
        </Show>
      </div>
    </div>
  );
};

export default MissionControl;
