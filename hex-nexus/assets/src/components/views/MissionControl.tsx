/**
 * MissionControl.tsx — hex-native operator console.
 *
 * NOT a Hermes clone. Built around hex's actual primitives:
 *   - factory roster (c-suite + IC personas with open-work counts)
 *   - live activity stream (twin verdicts + executor + autonomous commits
 *     interleaved chronologically — what the factory IS doing right now)
 *   - attention items (compact, in the rail; the wedge surface)
 *   - operator compose at the top — natural-language intent fired through
 *     `hex agent run` typed-tool loop
 *
 * One page. No drill-down footer. No tabbed history. Shows the system
 * doing its work.
 */

import { Component, For, Show, createSignal, onMount, onCleanup, createMemo } from "solid-js";
import { restClient } from "../../services/rest-client";

interface PersonaRow {
  role: string;
  display_name: string;
  paused: boolean;
  last_tick_at: string;
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

interface ActionRow {
  id: number;
  kind: string;
  proposed_by: string;
  status: string;
  twin_verdict: string;
  twin_rationale: string;
  escalate_reason: string;
}

interface CommitmentRow {
  id: number;
  role: string;
  action: string;
  success_artifact: string;
  status: string;
  created_at: string;
}

interface AttentionItem {
  id: string;
  priority: 0 | 1 | 2;
  kind: string;
  title: string;
  subtitle: string;
  age_seconds: number;
  action_url?: string;
  cli_repro?: string;
}

interface LiveEvent {
  id: number;
  event_type: string;
  created_at: string;
  session_id: string;
  preview: string;
}

interface Payload {
  stdb_alive: boolean;
  pulse?: { autonomous_commits_today?: number };
  personas: PersonaRow[];
  activity: { recent_executed: ExecutedRow[]; open_merge_requests: any[] };
  pending_decisions: { actions: ActionRow[]; commitments: CommitmentRow[]; anomalies: any[] };
  attention_feed?: AttentionItem[];
  live_events?: LiveEvent[];
}

const REFRESH_MS = 5000;

const ageSinceIso = (iso: string): string => {
  if (!iso) return "—";
  const t = Date.parse(iso);
  if (isNaN(t)) return "—";
  return ageSec(Math.max(0, Math.floor((Date.now() - t) / 1000)));
};

const ageSec = (s: number): string => {
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
};

// STDB occasionally hands us non-ISO timestamps wrapped in a Debug
// representation. Normalize so Date.parse works.
const tsToEpoch = (raw: any): number => {
  if (!raw) return 0;
  if (typeof raw === "number") return raw;
  const s = String(raw);
  const m = s.match(/__timestamp_micros_since_unix_epoch__:\s*(\d+)/);
  if (m) return Math.floor(parseInt(m[1], 10) / 1000);
  const t = Date.parse(s);
  return isNaN(t) ? 0 : t;
};

const ageSinceAny = (raw: any): string => {
  const ms = tsToEpoch(raw);
  if (!ms) return "—";
  return ageSec(Math.max(0, Math.floor((Date.now() - ms) / 1000)));
};

const truncate = (s: string, n: number): string =>
  s.length > n ? s.slice(0, n) + "…" : s;

const MODELS = [
  { id: "anthropic/claude-haiku-4.5", label: "haiku" },
  { id: "anthropic/claude-sonnet-4-6", label: "sonnet" },
  { id: "anthropic/claude-opus-4-7", label: "opus" },
];

const MissionControl: Component = () => {
  const [data, setData] = createSignal<Payload | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [intent, setIntent] = createSignal("");
  const [model, setModel] = createSignal(MODELS[0].id);
  const [maxIter, setMaxIter] = createSignal(6);
  const [running, setRunning] = createSignal(false);
  const [lastDispatch, setLastDispatch] = createSignal<string>("");

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
  onCleanup(() => { if (timer) clearInterval(timer); });

  const dispatch = async () => {
    const text = intent().trim();
    if (!text || running()) return;
    setRunning(true);
    setLastDispatch("dispatching…");
    try {
      const resp = await restClient.post("/api/agent/run", {
        intent: text,
        max_iterations: maxIter(),
        model: model(),
      });
      const steps = (resp?.steps || []).length;
      setLastDispatch(
        `${resp?.iterations ?? 0} iter · ${steps} step${steps === 1 ? "" : "s"} · ${resp?.stop_reason ?? "?"} · ${resp?.elapsed_ms ?? 0}ms`
      );
      setIntent("");
      await refresh();
    } catch (e: any) {
      setLastDispatch(`error: ${e?.message || String(e)}`);
    } finally {
      setRunning(false);
      setTimeout(() => setLastDispatch(""), 12000);
    }
  };

  // ── Derived: per-persona open work counts ────────────────────────
  const factoryRows = createMemo(() => {
    const d = data();
    if (!d) return [];
    const openByRole = new Map<string, number>();
    const escByRole = new Map<string, number>();
    for (const a of d.pending_decisions?.actions || []) {
      const by = a.proposed_by || "";
      // proposed_by may be "cto" or "tool:code_patch" — only count role-style
      if (by && !by.includes(":") && by !== "operator-passthrough") {
        openByRole.set(by, (openByRole.get(by) || 0) + 1);
        if (a.status === "escalated") escByRole.set(by, (escByRole.get(by) || 0) + 1);
      }
    }
    for (const c of d.pending_decisions?.commitments || []) {
      if (c.role && c.status !== "satisfied") {
        openByRole.set(c.role, (openByRole.get(c.role) || 0) + 1);
        if (c.status === "overdue" || c.status === "escalated")
          escByRole.set(c.role, (escByRole.get(c.role) || 0) + 1);
      }
    }
    return (d.personas || []).map((p) => ({
      ...p,
      open: openByRole.get(p.role) || 0,
      escalated: escByRole.get(p.role) || 0,
    }));
  });

  // ── Derived: unified activity stream (executed + events, sorted) ─
  interface ActivityItem {
    ts: number;
    icon: string;
    color: string;
    summary: string;
    detail: string;
    sourceId: string | number;
  }

  const activity = createMemo<ActivityItem[]>(() => {
    const d = data();
    if (!d) return [];
    const items: ActivityItem[] = [];
    for (const ex of d.activity?.recent_executed || []) {
      const ts = tsToEpoch(ex.executed_at);
      if (!ts) continue;
      const path = ex.path || "";
      items.push({
        ts,
        icon: ex.success ? "✎" : "✗",
        color: ex.success ? "text-cyan-300" : "text-red-400",
        summary: ex.success ? `wrote ${path.split("/").pop() || path}` : `${ex.kind} failed`,
        detail: ex.success
          ? `${ex.kind} · ${path} · action#${ex.id}`
          : `action#${ex.id} · ${ex.error || "no detail"}`,
        sourceId: ex.id,
      });
    }
    for (const ev of d.live_events || []) {
      const ts = tsToEpoch(ev.created_at);
      if (!ts) continue;
      const info = eventDecorate(ev.event_type);
      items.push({
        ts,
        icon: info.icon,
        color: info.color,
        summary: info.summary,
        detail: ev.preview ? truncate(ev.preview, 140) : ev.event_type,
        sourceId: `ev-${ev.id}`,
      });
    }
    items.sort((a, b) => b.ts - a.ts);
    return items.slice(0, 40);
  });

  const attention = createMemo(() => data()?.attention_feed || []);
  const p0 = () => attention().filter((i) => i.priority === 0).length;
  const p1 = () => attention().filter((i) => i.priority === 1).length;

  return (
    <div class="flex flex-col h-screen bg-zinc-950 text-zinc-100 font-sans">
      {/* ─── Header ─── */}
      <header class="px-6 py-3 border-b border-zinc-800 flex items-center justify-between flex-wrap gap-3">
        <div class="flex items-baseline gap-3">
          <h1 class="text-base font-semibold tracking-tight">hex</h1>
          <span class="text-[11px] text-zinc-500 font-mono">operator console</span>
        </div>
        <div class="flex items-center gap-3 text-[11px]">
          <span class={data()?.stdb_alive ? "text-green-400" : "text-red-400"}>
            STDB {data()?.stdb_alive ? "✓" : "✗"}
          </span>
          <span class="text-zinc-500">·</span>
          <span class="text-cyan-300 tabular-nums">
            {data()?.pulse?.autonomous_commits_today ?? "—"} commits today
          </span>
          <span class="text-zinc-500">·</span>
          <span class={p0() > 0 ? "text-red-400" : "text-zinc-500"}>{p0()} P0</span>
          <span class={p1() > 0 ? "text-amber-400" : "text-zinc-500"}>{p1()} P1</span>
          <span class="text-zinc-500">· refresh {REFRESH_MS / 1000}s</span>
        </div>
      </header>

      <Show when={error()}>
        <div class="px-6 py-2 bg-red-950/40 border-b border-red-900 text-red-300 text-xs">
          {error()}
        </div>
      </Show>

      {/* ─── Operator input ─── */}
      <div class="px-6 py-3 border-b border-zinc-800 bg-zinc-900/40">
        <div class="flex items-center gap-2 mb-1.5 text-[11px] text-zinc-500">
          <span>Tell the factory what you want.</span>
          <span class="ml-auto flex items-center gap-2">
            <select
              class="bg-zinc-900 border border-zinc-700 rounded px-1.5 py-0.5 text-zinc-300"
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
                class="w-10 bg-zinc-900 border border-zinc-700 rounded px-1 py-0.5 tabular-nums text-zinc-300"
                value={maxIter()}
                onInput={(e) => setMaxIter(Math.max(1, Math.min(20, parseInt(e.currentTarget.value) || 6)))}
                disabled={running()}
              />
            </label>
            <span class="text-zinc-600">⌘↵</span>
          </span>
        </div>
        <div class="flex gap-2">
          <input
            class="flex-1 bg-zinc-950 border border-zinc-700 focus:border-cyan-600 focus:outline-none rounded px-3 py-2 text-sm font-mono"
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
            class="px-5 rounded bg-cyan-700 hover:bg-cyan-600 text-white text-sm disabled:opacity-50"
            disabled={!intent().trim() || running()}
            onClick={dispatch}
          >
            {running() ? "Running…" : "Run"}
          </button>
        </div>
        <Show when={lastDispatch()}>
          <div class="text-[11px] text-zinc-400 mt-1.5 font-mono">{lastDispatch()}</div>
        </Show>
      </div>

      {/* ─── Two-column body ─── */}
      <div class="flex-1 grid grid-cols-12 gap-0 overflow-hidden">
        {/* Left rail: factory + attention */}
        <aside class="col-span-4 lg:col-span-3 border-r border-zinc-800 overflow-y-auto">
          <div class="px-4 py-3 border-b border-zinc-800">
            <h2 class="text-[10px] uppercase tracking-wide text-zinc-500 mb-2">Factory</h2>
            <div class="space-y-1">
              <For each={factoryRows()}>{(p) => (
                <div class="flex items-center gap-2 text-xs">
                  <span class={p.paused ? "text-yellow-400" : "text-green-400"}>●</span>
                  <span class="font-mono text-zinc-200 flex-1">{p.role}</span>
                  <Show when={p.escalated > 0}>
                    <span class="text-red-400 tabular-nums">{p.escalated}!</span>
                  </Show>
                  <Show when={p.open > 0 && p.escalated === 0}>
                    <span class="text-cyan-300 tabular-nums">{p.open}</span>
                  </Show>
                  <Show when={p.open === 0}>
                    <span class="text-zinc-600 tabular-nums">0</span>
                  </Show>
                  <span class="text-zinc-500 tabular-nums text-[10px] w-8 text-right">
                    {ageSinceAny(p.last_tick_at)}
                  </span>
                </div>
              )}</For>
              <Show when={factoryRows().length === 0 && !loading()}>
                <div class="text-zinc-500 text-xs italic">No personas registered.</div>
              </Show>
            </div>
          </div>

          <div class="px-4 py-3">
            <div class="flex items-center justify-between mb-2">
              <h2 class="text-[10px] uppercase tracking-wide text-zinc-500">Attention</h2>
              <span class="text-[10px] text-zinc-500 tabular-nums">{attention().length}</span>
            </div>
            <Show
              when={attention().length > 0}
              fallback={<div class="text-zinc-500 text-xs italic">Clear.</div>}
            >
              <div class="space-y-1.5">
                <For each={attention().slice(0, 15)}>{(item) => (
                  <div class="text-xs leading-tight">
                    <div class="flex items-center gap-1.5">
                      <span class={
                        item.priority === 0 ? "text-red-400" :
                        item.priority === 1 ? "text-amber-400" : "text-blue-400"
                      }>●</span>
                      <span class="text-zinc-300 truncate flex-1" title={item.title}>{item.title}</span>
                      <span class="text-zinc-600 tabular-nums shrink-0">{ageSec(item.age_seconds)}</span>
                    </div>
                  </div>
                )}</For>
                <Show when={attention().length > 15}>
                  <div class="text-[10px] text-zinc-500 italic pt-1">
                    … {attention().length - 15} more
                  </div>
                </Show>
              </div>
            </Show>
          </div>
        </aside>

        {/* Main: live activity stream */}
        <main class="col-span-8 lg:col-span-9 overflow-y-auto">
          <div class="px-6 py-3 border-b border-zinc-800 sticky top-0 bg-zinc-950 z-10">
            <h2 class="text-[10px] uppercase tracking-wide text-zinc-500">Live activity</h2>
          </div>
          <div class="px-6 py-3 space-y-1">
            <Show
              when={activity().length > 0}
              fallback={
                <div class="text-zinc-500 text-sm italic py-8 text-center">
                  Factory is quiet. Type an intent above to start something.
                </div>
              }
            >
              <For each={activity()}>{(item) => (
                <div class="flex items-baseline gap-3 text-xs py-1 border-b border-zinc-900/50 last:border-0">
                  <span class="text-zinc-500 tabular-nums w-10 shrink-0 text-right">
                    {ageSec(Math.max(0, Math.floor((Date.now() - item.ts) / 1000)))}
                  </span>
                  <span class={`${item.color} w-4 shrink-0`}>{item.icon}</span>
                  <span class="text-zinc-200 shrink-0 font-medium">{item.summary}</span>
                  <span class="text-zinc-500 truncate min-w-0 flex-1 font-mono">{item.detail}</span>
                </div>
              )}</For>
            </Show>
          </div>
        </main>
      </div>
    </div>
  );
};

function eventDecorate(evType: string): { icon: string; color: string; summary: string } {
  if (evType.startsWith("twin_") || evType === "twin_verdict") return { icon: "✓", color: "text-green-400", summary: "twin verdict" };
  if (evType === "executor_applied" || evType === "file_write") return { icon: "✎", color: "text-cyan-300", summary: "executor" };
  if (evType === "persona_reply") return { icon: "💬", color: "text-purple-300", summary: "persona reply" };
  if (evType === "thought_journaled") return { icon: "✦", color: "text-purple-300", summary: "thought" };
  if (evType.startsWith("improver_")) return { icon: "▼", color: "text-cyan-400", summary: "improver" };
  if (evType === "brain_tick") return { icon: "·", color: "text-zinc-500", summary: "brain tick" };
  if (evType.startsWith("commitment_")) return { icon: "↑", color: "text-amber-300", summary: "commitment" };
  if (evType.startsWith("escalat") || evType.startsWith("anomaly")) return { icon: "⚠", color: "text-red-400", summary: "anomaly" };
  return { icon: "·", color: "text-zinc-400", summary: evType };
}

export default MissionControl;
