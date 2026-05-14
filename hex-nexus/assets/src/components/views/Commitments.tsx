/**
 * Commitments.tsx — persona commitment ledger.
 *
 * Every Confirm/PLAN line a persona writes in chat is parsed and stored
 * in STDB `commitment`. This view shows them all with status (open /
 * overdue / satisfied / abandoned) so the operator can verify whether
 * the agent actually delivered.
 */

import { Component, For, Show, createSignal, onMount, onCleanup } from "solid-js";
import { restClient } from "../../services/rest-client";

interface CommitmentRow {
  id: number;
  role: string;
  raw_text: string;
  action: string;
  deadline_micros: number;
  success_artifact: string;
  artifact_kind: string; // verifiable_path | verifiable_route | operator_action | none
  thread_id: string;
  related_msg_id: number;
  created_at: string;
  status: string; // open | overdue | satisfied | abandoned
  last_checked: string;
  note: string;
}

const REFRESH_MS = 5000;

const statusBadge = (s: string) => {
  switch (s) {
    case "satisfied":
      return "bg-green-900 text-green-300 border-green-700";
    case "overdue":
      return "bg-red-900 text-red-300 border-red-700";
    case "open":
      return "bg-yellow-900 text-yellow-300 border-yellow-700";
    case "abandoned":
      return "bg-gray-800 text-gray-400 border-gray-700";
    default:
      return "bg-gray-800 text-gray-300 border-gray-700";
  }
};

const kindBadge = (k: string) => {
  switch (k) {
    case "verifiable_path":
      return { label: "file", cls: "text-cyan-400" };
    case "verifiable_route":
      return { label: "route", cls: "text-purple-400" };
    case "operator_action":
      return { label: "operator", cls: "text-yellow-400" };
    default:
      return { label: "vague", cls: "text-orange-400" };
  }
};

const fmtDeadline = (micros: number): string => {
  if (!micros || micros === 0) return "—";
  const ms = micros / 1000;
  const delta = ms - Date.now();
  const abs = Math.abs(delta) / 1000;
  if (abs < 60) return `${Math.floor(abs)}s ${delta > 0 ? "left" : "ago"}`;
  if (abs < 3600) return `${Math.floor(abs / 60)}m ${delta > 0 ? "left" : "ago"}`;
  if (abs < 86400) return `${Math.floor(abs / 3600)}h ${delta > 0 ? "left" : "ago"}`;
  return `${Math.floor(abs / 86400)}d ${delta > 0 ? "left" : "ago"}`;
};

const Commitments: Component = () => {
  const [commitments, setCommitments] = createSignal<CommitmentRow[]>([]);
  const [statusFilter, setStatusFilter] = createSignal<string>("all");
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [busyId, setBusyId] = createSignal<number | null>(null);

  let timer: ReturnType<typeof setInterval> | null = null;

  const refresh = async () => {
    try {
      const data = await restClient.get(`/api/commitments?status=${statusFilter()}&limit=200`);
      setCommitments(data.commitments || []);
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

  const satisfy = async (id: number) => {
    setBusyId(id);
    try {
      await restClient.post("/api/commitments/satisfy", {
        id,
        evidence: "operator marked satisfied via dashboard",
      });
      await refresh();
    } catch (e: any) {
      setError(`satisfy failed: ${e?.message || String(e)}`);
    } finally {
      setBusyId(null);
    }
  };

  const abandon = async (id: number) => {
    setBusyId(id);
    try {
      await restClient.post("/api/commitments/abandon", {
        id,
        reason: "operator abandoned via dashboard",
      });
      await refresh();
    } catch (e: any) {
      setError(`abandon failed: ${e?.message || String(e)}`);
    } finally {
      setBusyId(null);
    }
  };

  const counts = () => {
    const c = { open: 0, overdue: 0, satisfied: 0, abandoned: 0 };
    for (const x of commitments()) {
      if (x.status in c) (c as any)[x.status]++;
    }
    return c;
  };

  return (
    <div class="flex flex-col bg-gray-950 min-h-screen text-gray-100">
      <div class="p-6 border-b border-gray-800">
        <div class="flex items-baseline justify-between gap-4">
          <div>
            <h1 class="text-2xl font-bold mb-1">Commitments</h1>
            <p class="text-gray-400 text-sm">
              Every persona Confirm/PLAN line landed here. Overdue rows turn red after 1h grace.
            </p>
          </div>
          <div class="flex items-center gap-2">
            <span class="text-xs text-gray-500">{counts().open} open · </span>
            <span class="text-xs text-red-400">{counts().overdue} overdue · </span>
            <span class="text-xs text-green-400">{counts().satisfied} done</span>
            <select
              class="ml-3 bg-gray-900 border border-gray-800 rounded px-2 py-1 text-xs"
              value={statusFilter()}
              onChange={(e) => {
                setStatusFilter(e.currentTarget.value);
                setLoading(true);
                refresh();
              }}
            >
              <option value="active">active (open + overdue)</option>
              <option value="open">open only</option>
              <option value="overdue">overdue only</option>
              <option value="all">all</option>
            </select>
          </div>
        </div>
      </div>

      <Show when={error()}>
        <div class="p-4 bg-red-950/40 border-b border-red-900 text-red-300 text-sm">
          {error()}
        </div>
      </Show>

      <Show when={loading() && commitments().length === 0}>
        <div class="p-6 text-gray-500">Loading commitments…</div>
      </Show>

      <Show when={!loading() && commitments().length === 0}>
        <div class="p-6 text-gray-500">
          No commitments under the current filter. The personas haven't agreed to anything verifiable yet.
        </div>
      </Show>

      <div class="flex-1 overflow-y-auto px-6 py-4 space-y-3">
        <For each={commitments()}>
          {(c) => {
            const k = kindBadge(c.artifact_kind);
            const isOpen = c.status === "open" || c.status === "overdue";
            return (
              <div class="border border-gray-800 rounded bg-gray-900/40 p-4">
                <div class="flex items-start justify-between gap-3">
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-2 flex-wrap">
                      <span class="text-cyan-400 font-mono">{c.role}</span>
                      <span
                        class={`px-2 py-0.5 rounded text-xs border ${statusBadge(c.status)}`}
                      >
                        {c.status}
                      </span>
                      <span class={`text-xs ${k.cls}`}>artifact: {k.label}</span>
                      <span class="text-xs text-gray-500">deadline: {fmtDeadline(c.deadline_micros)}</span>
                      <span class="ml-auto text-xs text-gray-600">#{c.id}</span>
                    </div>
                    <div class="mt-2 text-gray-100">{c.action}</div>
                    <Show when={c.success_artifact}>
                      <div class="mt-1 text-gray-400 text-sm">
                        <span class="text-gray-500">success: </span>
                        <span class="font-mono">{c.success_artifact}</span>
                      </div>
                    </Show>
                    <Show when={c.note}>
                      <div class="mt-1 text-gray-500 text-xs">{c.note}</div>
                    </Show>
                    <details class="mt-2">
                      <summary class="text-gray-600 text-xs cursor-pointer hover:text-gray-400">raw</summary>
                      <pre class="text-xs text-gray-500 mt-1 whitespace-pre-wrap font-mono">{c.raw_text}</pre>
                    </details>
                  </div>
                  <Show when={isOpen}>
                    <div class="flex flex-col gap-1 shrink-0">
                      <button
                        class="px-3 py-1 rounded bg-green-700 hover:bg-green-600 text-white text-xs disabled:opacity-50"
                        disabled={busyId() === c.id}
                        onClick={() => satisfy(c.id)}
                      >
                        Satisfied
                      </button>
                      <button
                        class="px-3 py-1 rounded bg-gray-700 hover:bg-gray-600 text-white text-xs disabled:opacity-50"
                        disabled={busyId() === c.id}
                        onClick={() => abandon(c.id)}
                      >
                        Abandon
                      </button>
                    </div>
                  </Show>
                </div>
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
};

export default Commitments;
