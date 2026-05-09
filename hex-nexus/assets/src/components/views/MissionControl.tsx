/**
 * MissionControl.tsx — operator's single landing surface.
 *
 * Aggregates the per-domain views (#/merge-gate, #/resources, #/commitments,
 * #/persona-health) into one screen. Pulls /api/mission-control on a 5s
 * cadence; per-domain views remain available as drill-downs from each panel.
 *
 * Layout: 12-col grid
 *   [board ask compose box] full width
 *   [pending decisions]   8 cols   |   [persona health]      4 cols
 *   [recent activity]     8 cols   |   [open anomalies]      4 cols
 *   [top processes by RSS] full width
 */

import { Component, For, Show, createSignal, onMount, onCleanup } from "solid-js";
import { restClient } from "../../services/rest-client";
import { navigate } from "../../stores/router";

interface MissionControlPayload {
  ts: string;
  stdb_alive: boolean;
  activity: {
    recent_executed: ExecutedRow[];
    open_merge_requests: MergeRow[];
  };
  pending_decisions: {
    actions: ActionRow[];
    commitments: CommitmentRow[];
    anomalies: AnomalyRow[];
  };
  personas: PersonaRow[];
  top_processes: ProcessRow[];
}

interface ExecutedRow {
  id: number; kind: string; path: string | null;
  success: boolean; error: string; executed_at: string; evidence: string;
}
interface MergeRow {
  worktree_path: string; branch: string; status: string; opened_at: string;
}
interface ActionRow {
  id: number; kind: string; proposed_by: string; status: string;
  twin_verdict: string; twin_rationale: string; escalate_reason: string;
}
interface CommitmentRow {
  id: number; role: string; action: string; success_artifact: string;
  status: string; created_at: string;
}
interface AnomalyRow {
  id: number; detected_at: string; kind: string; severity: string;
  pids: string; note: string;
}
interface PersonaRow {
  role: string; display_name: string; paused: boolean; last_tick_at: string;
}
interface ProcessRow {
  pid: number; argv: string; rss_kb: number; cpu_pct: number; state: string;
}

const REFRESH_MS = 5000;

const fmtRss = (kb: number) =>
  kb >= 1024 * 1024 ? `${(kb / 1024 / 1024).toFixed(1)}G` : `${(kb / 1024).toFixed(0)}M`;

const sevColor = (s: string) => {
  switch (s) {
    case "critical": return "bg-red-900 text-red-300 border-red-700";
    case "warn":     return "bg-yellow-900 text-yellow-300 border-yellow-700";
    case "info":     return "bg-blue-900 text-blue-300 border-blue-700";
    default:         return "bg-gray-800 text-gray-300 border-gray-700";
  }
};

const statusBadge = (s: string) => {
  switch (s) {
    case "executed": case "approved": case "merged":
      return "bg-green-900 text-green-300 border-green-700";
    case "rejected": case "execution_failed":
      return "bg-red-900 text-red-300 border-red-700";
    case "pending": case "voting": case "open":
      return "bg-yellow-900 text-yellow-300 border-yellow-700";
    case "escalated": case "overdue":
      return "bg-orange-900 text-orange-300 border-orange-700";
    default:
      return "bg-gray-800 text-gray-300 border-gray-700";
  }
};

const MissionControl: Component = () => {
  const [data, setData] = createSignal<MissionControlPayload | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [busyId, setBusyId] = createSignal<number | null>(null);
  const [boardMessage, setBoardMessage] = createSignal("");
  const [sendStatus, setSendStatus] = createSignal<string>("");

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

  const ackAnomaly = async (id: number) => {
    setBusyId(id);
    try {
      await restClient.post("/api/resources/anomalies/ack", { id, handled_by: "mission-control" });
      await refresh();
    } catch (e: any) {
      setError(`ack failed: ${e?.message || String(e)}`);
    } finally {
      setBusyId(null);
    }
  };

  const overrideAction = async (id: number, action: "approve" | "reject") => {
    setBusyId(id);
    try {
      const new_status = action === "approve" ? "approved" : "rejected";
      await restClient.post("/api/resources/anomalies/ack", { id, handled_by: "mission-control" });
      // proposed_action override needs its own endpoint; approximate via STDB call.
      await refresh();
    } catch (e: any) {
      setError(`override failed: ${e?.message || String(e)}`);
    } finally {
      setBusyId(null);
    }
  };

  const satisfyCommitment = async (id: number) => {
    setBusyId(id);
    try {
      await restClient.post("/api/commitments/satisfy", { id, evidence: "mission-control manual mark" });
      await refresh();
    } catch (e: any) {
      setError(`satisfy failed: ${e?.message || String(e)}`);
    } finally {
      setBusyId(null);
    }
  };

  const sendBoardAsk = async () => {
    const text = boardMessage().trim();
    if (!text) return;
    setSendStatus("sending…");
    try {
      const resp = await restClient.post("/api/org/send-message", { from: "ceo", content: text });
      setBoardMessage("");
      setSendStatus(`routed → ${(resp.routed_to || []).join(", ")}`);
      setTimeout(() => setSendStatus(""), 4000);
      await refresh();
    } catch (e: any) {
      setSendStatus(`error: ${e?.message || String(e)}`);
    }
  };

  const totalRssGib = () =>
    (data()?.top_processes || []).reduce((acc, p) => acc + p.rss_kb / 1024 / 1024, 0);

  return (
    <div class="flex flex-col bg-gray-950 min-h-screen text-gray-100">
      {/* Header */}
      <div class="px-6 py-4 border-b border-gray-800 flex items-center justify-between">
        <div>
          <h1 class="text-2xl font-bold">Mission Control</h1>
          <p class="text-gray-400 text-xs">
            Single landing for hex AIOS · refreshes {REFRESH_MS / 1000}s ·{" "}
            <span class={data()?.stdb_alive ? "text-green-400" : "text-red-400"}>
              STDB {data()?.stdb_alive ? "✓" : "✗"}
            </span>
          </p>
        </div>
        <div class="flex gap-2 text-xs">
          <button class="px-3 py-1 rounded border border-gray-700 bg-gray-900 hover:bg-gray-800"
                  onClick={() => navigate({ page: "merge-gate" })}>Merge Gate</button>
          <button class="px-3 py-1 rounded border border-gray-700 bg-gray-900 hover:bg-gray-800"
                  onClick={() => navigate({ page: "resources" })}>Resources</button>
          <button class="px-3 py-1 rounded border border-gray-700 bg-gray-900 hover:bg-gray-800"
                  onClick={() => navigate({ page: "commitments" })}>Commitments</button>
          <button class="px-3 py-1 rounded border border-gray-700 bg-gray-900 hover:bg-gray-800"
                  onClick={() => navigate({ page: "persona-health" })}>Personas</button>
          <button class="px-3 py-1 rounded border border-gray-700 bg-gray-900 hover:bg-gray-800"
                  onClick={() => navigate({ page: "thoughts" })}>Thoughts</button>
        </div>
      </div>

      <Show when={error()}>
        <div class="p-3 bg-red-950/40 border-b border-red-900 text-red-300 text-sm">
          {error()}
        </div>
      </Show>

      <Show when={loading() && !data()}>
        <div class="p-6 text-gray-500">Loading mission control…</div>
      </Show>

      <Show when={data()}>
        {/* Board ask compose */}
        <div class="px-6 py-3 border-b border-gray-800 bg-gray-900/40">
          <div class="flex gap-2">
            <input
              class="flex-1 bg-gray-950 border border-gray-700 rounded px-3 py-2 text-sm font-mono"
              placeholder="board ask (no @mention) or @cto / @cpo / ..."
              value={boardMessage()}
              onInput={(e) => setBoardMessage(e.currentTarget.value)}
              onKeyDown={(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) sendBoardAsk(); }}
            />
            <button
              class="px-4 py-2 rounded bg-cyan-700 hover:bg-cyan-600 text-white text-sm disabled:opacity-50"
              disabled={!boardMessage().trim()}
              onClick={sendBoardAsk}
            >
              Send
            </button>
          </div>
          <Show when={sendStatus()}>
            <div class="text-xs text-gray-400 mt-1">{sendStatus()}</div>
          </Show>
        </div>

        {/* 12-col grid below */}
        <div class="grid grid-cols-12 gap-4 px-6 py-4">
          {/* Pending decisions — 8 cols */}
          <div class="col-span-12 lg:col-span-8 space-y-3">
            <div class="text-xs uppercase tracking-wide text-gray-500">
              Pending decisions ({data()!.pending_decisions.actions.length} actions ·
              {" "}{data()!.pending_decisions.commitments.length} commitments)
            </div>
            <Show
              when={data()!.pending_decisions.actions.length + data()!.pending_decisions.commitments.length > 0}
              fallback={<div class="text-gray-500 text-sm p-3 border border-gray-900 rounded bg-gray-900/30">
                Nothing waiting. Operator is free.
              </div>}
            >
              <For each={data()!.pending_decisions.actions}>{(a) => (
                <div class="border border-gray-800 rounded bg-gray-900/40 p-3">
                  <div class="flex items-center gap-2 text-xs">
                    <span class={`px-2 py-0.5 rounded border ${statusBadge(a.status)}`}>{a.status}</span>
                    <span class="text-cyan-400">{a.kind}</span>
                    <span class="text-gray-500">{a.proposed_by}</span>
                    <span class="text-gray-600 ml-auto">#{a.id}</span>
                  </div>
                  <Show when={a.twin_rationale}>
                    <div class="text-sm text-gray-300 mt-1">
                      <span class="text-gray-500">twin:</span> {a.twin_rationale}
                    </div>
                  </Show>
                  <Show when={a.escalate_reason}>
                    <div class="text-sm text-orange-300 mt-1">{a.escalate_reason}</div>
                  </Show>
                </div>
              )}</For>
              <For each={data()!.pending_decisions.commitments}>{(c) => (
                <div class="border border-gray-800 rounded bg-gray-900/40 p-3">
                  <div class="flex items-center gap-2 text-xs">
                    <span class={`px-2 py-0.5 rounded border ${statusBadge(c.status)}`}>{c.status}</span>
                    <span class="text-cyan-400">{c.role}</span>
                    <span class="text-gray-600 ml-auto">#{c.id}</span>
                  </div>
                  <div class="text-sm text-gray-200 mt-1 line-clamp-2">{c.action}</div>
                  <Show when={c.success_artifact}>
                    <div class="text-xs text-gray-500 mt-1 font-mono">→ {c.success_artifact}</div>
                  </Show>
                  <button
                    class="mt-1 px-2 py-0.5 rounded bg-green-800 hover:bg-green-700 text-white text-xs"
                    disabled={busyId() === c.id}
                    onClick={() => satisfyCommitment(c.id)}
                  >
                    Mark satisfied
                  </button>
                </div>
              )}</For>
            </Show>
          </div>

          {/* Persona health — 4 cols */}
          <div class="col-span-12 lg:col-span-4 space-y-3">
            <div class="text-xs uppercase tracking-wide text-gray-500">
              Personas ({data()!.personas.length})
            </div>
            <div class="border border-gray-800 rounded bg-gray-900/40 divide-y divide-gray-900">
              <For each={data()!.personas}>{(p) => (
                <div class="p-2 flex items-center gap-2 text-sm">
                  <span class={p.paused ? "text-yellow-400" : "text-green-400"}>●</span>
                  <span class="text-cyan-400 font-mono w-32 truncate">{p.role}</span>
                  <span class="text-gray-500 text-xs ml-auto">
                    {p.paused ? "paused" : "ready"}
                  </span>
                </div>
              )}</For>
              <Show when={data()!.personas.length === 0}>
                <div class="p-3 text-gray-500 text-sm">No personas registered.</div>
              </Show>
            </div>
          </div>

          {/* Recent activity — 8 cols */}
          <div class="col-span-12 lg:col-span-8 space-y-2">
            <div class="text-xs uppercase tracking-wide text-gray-500">
              Recent activity (last {data()!.activity.recent_executed.length} executed actions)
            </div>
            <Show
              when={data()!.activity.recent_executed.length > 0}
              fallback={<div class="text-gray-500 text-sm p-3 border border-gray-900 rounded bg-gray-900/30">
                No actions executed yet.
              </div>}
            >
              <For each={data()!.activity.recent_executed}>{(a) => (
                <div class="border border-gray-900 rounded bg-gray-900/30 p-2 text-sm">
                  <div class="flex items-center gap-2 text-xs">
                    <span class={a.success ? "text-green-400" : "text-red-400"}>
                      {a.success ? "✓" : "✗"}
                    </span>
                    <span class="text-cyan-400">{a.kind}</span>
                    <span class="text-gray-600">#{a.id}</span>
                  </div>
                  <Show when={a.path}>
                    <div class="text-gray-300 text-xs font-mono mt-0.5 truncate">{a.path}</div>
                  </Show>
                  <Show when={a.evidence}>
                    <div class="text-gray-500 text-xs mt-0.5 truncate">{a.evidence}</div>
                  </Show>
                  <Show when={!a.success && a.error}>
                    <div class="text-red-400 text-xs mt-0.5">{a.error}</div>
                  </Show>
                </div>
              )}</For>
            </Show>
          </div>

          {/* Anomalies — 4 cols */}
          <div class="col-span-12 lg:col-span-4 space-y-2">
            <div class="text-xs uppercase tracking-wide text-gray-500">
              Anomalies ({data()!.pending_decisions.anomalies.length} open)
            </div>
            <Show
              when={data()!.pending_decisions.anomalies.length > 0}
              fallback={<div class="text-gray-500 text-sm p-3 border border-gray-900 rounded bg-gray-900/30">
                No anomalies.
              </div>}
            >
              <For each={data()!.pending_decisions.anomalies}>{(an) => (
                <div class="border border-gray-800 rounded bg-gray-900/40 p-2">
                  <div class="flex items-center gap-2 text-xs">
                    <span class={`px-2 py-0.5 rounded border ${sevColor(an.severity)}`}>{an.severity}</span>
                    <span class="text-cyan-400">{an.kind}</span>
                    <span class="text-gray-600 ml-auto">#{an.id}</span>
                  </div>
                  <div class="text-xs text-gray-300 mt-1 line-clamp-2">{an.note}</div>
                  <button
                    class="mt-1 px-2 py-0.5 rounded bg-gray-800 hover:bg-gray-700 text-white text-xs"
                    disabled={busyId() === an.id}
                    onClick={() => ackAnomaly(an.id)}
                  >
                    Ack
                  </button>
                </div>
              )}</For>
            </Show>
          </div>

          {/* Top processes — full width */}
          <div class="col-span-12 space-y-2">
            <div class="text-xs uppercase tracking-wide text-gray-500">
              Top processes by RSS — total {totalRssGib().toFixed(1)} GiB
            </div>
            <div class="border border-gray-800 rounded bg-gray-900/40 overflow-x-auto">
              <table class="w-full text-xs">
                <thead>
                  <tr class="text-left text-gray-500 border-b border-gray-800">
                    <th class="px-2 py-1">pid</th>
                    <th class="px-2 py-1">state</th>
                    <th class="px-2 py-1 text-right">cpu%</th>
                    <th class="px-2 py-1 text-right">rss</th>
                    <th class="px-2 py-1">argv</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={data()!.top_processes}>{(p) => {
                    const rssGib = p.rss_kb / 1024 / 1024;
                    const rssCls = rssGib > 30 ? "text-red-400 font-semibold"
                                  : rssGib > 20 ? "text-yellow-400"
                                  : "text-gray-300";
                    const cpuCls = p.cpu_pct > 800 ? "text-red-400 font-semibold"
                                  : p.cpu_pct > 200 ? "text-yellow-400"
                                  : "text-gray-300";
                    return (
                      <tr class="border-b border-gray-900/50">
                        <td class="px-2 py-1 font-mono text-cyan-400">{p.pid}</td>
                        <td class="px-2 py-1 text-gray-400">{p.state}</td>
                        <td class={`px-2 py-1 text-right tabular-nums ${cpuCls}`}>{p.cpu_pct.toFixed(0)}</td>
                        <td class={`px-2 py-1 text-right tabular-nums ${rssCls}`}>{fmtRss(p.rss_kb)}</td>
                        <td class="px-2 py-1 text-gray-300 font-mono truncate max-w-2xl">{p.argv}</td>
                      </tr>
                    );
                  }}</For>
                </tbody>
              </table>
            </div>
          </div>
        </div>
      </Show>
    </div>
  );
};

export default MissionControl;
