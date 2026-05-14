/**
 * MissionControl.tsx — operator's single landing surface.
 *
 * One dense page, top to bottom:
 *   [stats strip]   STDB · today's auto-commits · P0/P1/P2 counts
 *   [agent run]     compose box dispatching to /api/agent/run
 *   [attention]     AttentionFeed (the wedge surface)
 *   [auto-commits]  recent autonomous commit stream
 *   [personas]      one-line health rail
 *   [drill-downs]   compact footer links to per-domain views
 */

import { Component, For, Show, createSignal, onMount, onCleanup } from "solid-js";
import { restClient } from "../../services/rest-client";
import { navigate } from "../../stores/router";
import AttentionFeed from "./AttentionFeed";

interface AttentionItem {
  id: string;
  priority: 0 | 1 | 2;
  kind: 'escalation' | 'overdue_commitment' | 'merge_vote_needed' | 'resource_anomaly' | 'autonomous_commit' | 'agent_run_active';
  title: string;
  subtitle: string;
  age_seconds: number;
  action_url?: string;
  cli_repro?: string;
}

interface MissionControlPayload {
  ts: string;
  stdb_alive: boolean;
  attention_feed?: AttentionItem[];
  pulse?: PulseRow;
  activity: { recent_executed: ExecutedRow[]; open_merge_requests: any[] };
  pending_decisions: { actions: any[]; commitments: any[]; anomalies: any[] };
  personas: PersonaRow[];
  top_processes: ProcessRow[];
}

interface PulseRow {
  last_thought_ts: string;
  last_persona_role: string;
  last_persona_msg_ts: string;
  last_persona_msg_preview: string;
  last_improver_event_ts: string;
  total_thoughts_db: number;
  active_pattern_count: number;
  git_head: { sha: string; subject: string; age_seconds: number };
  autonomous_commits_today?: number;
}

interface ExecutedRow {
  id: number;
  kind: string;
  path: string | null;
  success: boolean;
  error: string;
  executed_at: string;
  evidence: string;
}

interface PersonaRow {
  role: string;
  display_name: string;
  paused: boolean;
  last_tick_at: string;
}

interface ProcessRow {
  pid: number;
  argv: string;
  rss_kb: number;
  cpu_pct: number;
  state: string;
}

const REFRESH_MS = 5000;

const ageSince = (iso: string): string => {
  if (!iso) return "—";
  const t = Date.parse(iso);
  if (isNaN(t)) return "—";
  const s = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
};

const MissionControl: Component = () => {
  const [data, setData] = createSignal<MissionControlPayload | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [intent, setIntent] = createSignal("");
  const [runStatus, setRunStatus] = createSignal<string>("");
  const [running, setRunning] = createSignal(false);

  let timer: ReturnType<typeof setInterval> | null = null;

  const refresh = async () => {
    try {
      const d = await restClient.get("/api/mission-control");
      setData(d);
      setError(null);
    } catch (e: any) {
      setError(e?.message || String(e));
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    refresh();
    timer = setInterval(refresh, REFRESH_MS);
  });
  onCleanup(() => {
    if (timer) clearInterval(timer);
  });

  const dispatchAgentRun = async () => {
    const text = intent().trim();
    if (!text || running()) return;
    setRunning(true);
    setRunStatus("dispatching…");
    try {
      const resp = await restClient.post("/api/agent/run", {
        intent: text,
        max_iterations: 6,
        model: "anthropic/claude-haiku-4.5",
      });
      setIntent("");
      const r = resp || {};
      setRunStatus(
        `done · ${r.iterations || 0} iter · ${(r.steps || []).length} steps · ${r.stop_reason || "?"}`
      );
      await refresh();
    } catch (e: any) {
      setRunStatus(`error: ${e?.message || String(e)}`);
    } finally {
      setRunning(false);
      setTimeout(() => setRunStatus(""), 8000);
    }
  };

  const af = () => data()?.attention_feed || [];
  const p0 = () => af().filter((i) => i.priority === 0).length;
  const p1 = () => af().filter((i) => i.priority === 1).length;
  const p2 = () => af().filter((i) => i.priority === 2).length;

  const recentAutoCommits = () =>
    (data()?.activity?.recent_executed || [])
      .filter((e) => e.success && e.path)
      .slice(0, 10);

  const drillDowns: Array<{ label: string; page: string }> = [
    { label: "Resources", page: "resources" },
    { label: "Personas", page: "persona-health" },
    { label: "Commitments", page: "commitments" },
    { label: "Merge Gate", page: "merge-gate" },
    { label: "Brain", page: "brain" },
    { label: "Agent Runs", page: "agent-runs" },
    { label: "Org Chart", page: "org-chart" },
    { label: "Thoughts", page: "thoughts" },
    { label: "Missions", page: "missions" },
  ];

  return (
    <div class="flex flex-col bg-gray-950 min-h-screen text-gray-100">
      {/* Stats strip */}
      <div class="px-6 py-3 border-b border-gray-800 flex items-center justify-between flex-wrap gap-2">
        <div class="flex items-baseline gap-3">
          <h1 class="text-xl font-bold">Mission Control</h1>
          <span class="text-xs text-gray-500">refresh {REFRESH_MS / 1000}s</span>
        </div>
        <div class="flex items-center gap-2 text-xs">
          <span class={`px-2 py-1 rounded border ${data()?.stdb_alive ? "border-green-700 bg-green-900/30 text-green-300" : "border-red-700 bg-red-900/30 text-red-300"}`}>
            STDB {data()?.stdb_alive ? "✓" : "✗"}
          </span>
          <span class="px-2 py-1 rounded border border-cyan-700 bg-cyan-900/30 text-cyan-300 tabular-nums">
            {data()?.pulse?.autonomous_commits_today ?? "—"} auto-commits today
          </span>
          <span class={`px-2 py-1 rounded border tabular-nums ${p0() > 0 ? "border-red-700 bg-red-900/30 text-red-300" : "border-gray-700 bg-gray-900 text-gray-500"}`}>
            {p0()} P0
          </span>
          <span class={`px-2 py-1 rounded border tabular-nums ${p1() > 0 ? "border-amber-700 bg-amber-900/30 text-amber-300" : "border-gray-700 bg-gray-900 text-gray-500"}`}>
            {p1()} P1
          </span>
          <span class="px-2 py-1 rounded border border-gray-700 bg-gray-900 text-gray-500 tabular-nums">
            {p2()} P2
          </span>
        </div>
      </div>

      <Show when={error()}>
        <div class="px-6 py-2 bg-red-950/40 border-b border-red-900 text-red-300 text-sm">
          {error()}
        </div>
      </Show>

      <Show when={loading() && !data()}>
        <div class="p-6 text-gray-500">Loading…</div>
      </Show>

      <Show when={data()}>
        {/* Compose: hex agent run from the dashboard */}
        <div class="px-6 py-3 border-b border-gray-800 bg-gray-900/30">
          <div class="flex items-center gap-2 text-xs text-gray-400 mb-1">
            <span class="font-mono text-cyan-400">hex agent run</span>
            <span>natural-language intent, dispatched via typed-tool loop</span>
          </div>
          <div class="flex gap-2">
            <input
              class="flex-1 bg-gray-950 border border-gray-700 rounded px-3 py-2 text-sm font-mono"
              placeholder='e.g. "use code_patch to create docs/specs/hello.md with new_content ..."'
              value={intent()}
              onInput={(e) => setIntent(e.currentTarget.value)}
              onKeyDown={(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) dispatchAgentRun(); }}
              disabled={running()}
            />
            <button
              class="px-4 py-2 rounded bg-cyan-700 hover:bg-cyan-600 text-white text-sm disabled:opacity-50"
              disabled={!intent().trim() || running()}
              onClick={dispatchAgentRun}
            >
              {running() ? "Running…" : "Run"}
            </button>
          </div>
          <Show when={runStatus()}>
            <div class="text-xs text-gray-400 mt-1 font-mono">{runStatus()}</div>
          </Show>
        </div>

        {/* Attention feed — the wedge surface */}
        <div class="px-6 pt-4">
          <h2 class="text-xs uppercase tracking-wide text-gray-500 mb-2">Attention</h2>
          <Show
            when={af().length > 0}
            fallback={
              <div class="text-gray-500 text-sm p-3 border border-gray-900 rounded bg-gray-900/30">
                Nothing waiting. Operator is free.
              </div>
            }
          >
            <AttentionFeed items={af()} />
          </Show>
        </div>

        {/* Recent autonomous commits */}
        <Show when={recentAutoCommits().length > 0}>
          <div class="px-6 pt-4">
            <h2 class="text-xs uppercase tracking-wide text-gray-500 mb-2">Recent autonomous activity</h2>
            <div class="space-y-1 text-xs">
              <For each={recentAutoCommits()}>{(ex) => (
                <div class="flex items-center gap-3 px-3 py-1.5 rounded border border-gray-900 bg-gray-950 hover:bg-gray-900/50">
                  <span class="font-mono text-cyan-400 shrink-0">#{ex.id}</span>
                  <span class="text-gray-400 shrink-0">{ex.kind}</span>
                  <span class="font-mono text-gray-200 truncate flex-1">{ex.path}</span>
                  <span class="text-gray-500 shrink-0">{ageSince(ex.executed_at)}</span>
                </div>
              )}</For>
            </div>
          </div>
        </Show>

        {/* Persona health rail */}
        <Show when={data()!.personas.length > 0}>
          <div class="px-6 pt-4">
            <h2 class="text-xs uppercase tracking-wide text-gray-500 mb-2">Personas</h2>
            <div class="flex flex-wrap gap-2 text-xs">
              <For each={data()!.personas}>{(p) => (
                <div class="flex items-center gap-1.5 px-2 py-1 rounded border border-gray-800 bg-gray-900/40">
                  <span class={p.paused ? "text-yellow-400" : "text-green-400"}>●</span>
                  <span class="font-mono text-gray-200">{p.role}</span>
                  <span class="text-gray-500">{ageSince(p.last_tick_at)}</span>
                </div>
              )}</For>
            </div>
          </div>
        </Show>

        {/* Drill-down footer */}
        <div class="px-6 py-4 mt-4 border-t border-gray-800 text-xs">
          <span class="text-gray-500">Drill-downs:</span>
          <For each={drillDowns}>{(d) => (
            <button
              class="ml-3 text-gray-400 hover:text-cyan-400"
              onClick={() => navigate({ page: d.page } as any)}
            >
              {d.label}
            </button>
          )}</For>
        </div>
      </Show>
    </div>
  );
};

export default MissionControl;
