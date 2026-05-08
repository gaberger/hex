/**
 * MergeGate.tsx — operator dashboard for ADR-2605081126.
 *
 * Mirrors `hex worktree status` and the merge_gate REST surface. Lists every
 * merge_request in STDB with its three voters (validation-judge,
 * adversarial-red, adversarial-blue), shows their pass/fail/abstain verdicts,
 * and gives the operator approve / reject buttons that vote as `operator`.
 */

import { Component, For, Show, createSignal, onMount, onCleanup } from "solid-js";
import { restClient } from "../../services/rest-client";

interface MergeVote {
  voter: string;
  verdict: string;   // "pass" | "fail" | "abstain"
  reason: string;
  voted_at: string;
}

interface MergeRequest {
  worktree_path: string;
  branch: string;
  role: string;
  opened_at: string;
  status: string;     // "open" | "merged" | "rejected"
  related_workplan: string;
  agent_id: string;
  votes: MergeVote[];
}

interface MergeRequestList {
  requests: MergeRequest[];
}

const REFRESH_MS = 4000;

const REQUIRED_VOTERS = ["validation-judge", "adversarial-red", "adversarial-blue"];

const statusBadge = (s: string) => {
  switch (s) {
    case "merged":
      return "bg-green-900 text-green-300 border-green-700";
    case "rejected":
      return "bg-red-900 text-red-300 border-red-700";
    case "open":
      return "bg-yellow-900 text-yellow-300 border-yellow-700";
    default:
      return "bg-gray-800 text-gray-300 border-gray-700";
  }
};

const verdictColor = (v: string) => {
  switch (v) {
    case "pass":
      return "text-green-400";
    case "fail":
      return "text-red-400";
    case "abstain":
      return "text-gray-500";
    default:
      return "text-yellow-400";
  }
};

const tally = (req: MergeRequest) => {
  let pass = 0,
    fail = 0,
    abstain = 0;
  for (const v of req.votes) {
    if (v.verdict === "pass") pass++;
    else if (v.verdict === "fail") fail++;
    else if (v.verdict === "abstain") abstain++;
  }
  return { pass, fail, abstain };
};

const MergeGate: Component = () => {
  const [requests, setRequests] = createSignal<MergeRequest[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [busyPath, setBusyPath] = createSignal<string | null>(null);
  const [rejectReason, setRejectReason] = createSignal<Record<string, string>>({});

  let timer: ReturnType<typeof setInterval> | null = null;

  const refresh = async () => {
    try {
      const data: MergeRequestList = await restClient.get("/api/merge/requests");
      setRequests(data.requests || []);
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

  const approve = async (path: string) => {
    setBusyPath(path);
    try {
      await restClient.post("/api/merge/approve", {
        worktree_path: path,
        reason: "operator approval",
      });
      await refresh();
    } catch (e: any) {
      setError(`approve failed: ${e?.message || String(e)}`);
    } finally {
      setBusyPath(null);
    }
  };

  const reject = async (path: string) => {
    const reason = (rejectReason()[path] || "").trim();
    if (!reason) {
      setError("reject requires a reason");
      return;
    }
    setBusyPath(path);
    try {
      await restClient.post("/api/merge/reject", { worktree_path: path, reason });
      await refresh();
    } catch (e: any) {
      setError(`reject failed: ${e?.message || String(e)}`);
    } finally {
      setBusyPath(null);
    }
  };

  return (
    <div class="flex flex-col bg-gray-950 min-h-screen text-gray-100">
      <div class="p-6 border-b border-gray-800">
        <div class="flex items-baseline justify-between">
          <div>
            <h1 class="text-2xl font-bold mb-1">Merge Gate</h1>
            <p class="text-gray-400 text-sm">
              ADR-2605081126 · validation-judge · adversarial-red · adversarial-blue
            </p>
          </div>
          <div class="text-xs text-gray-500">refresh {REFRESH_MS / 1000}s</div>
        </div>
      </div>

      <Show when={error()}>
        <div class="p-4 bg-red-950/40 border-b border-red-900 text-red-300 text-sm">
          {error()}
        </div>
      </Show>

      <Show when={loading()}>
        <div class="p-6 text-gray-500">Loading merge queue…</div>
      </Show>

      <Show when={!loading() && requests().length === 0}>
        <div class="p-6 text-gray-500">
          No merge requests on record. Trunk is quiet — voters have nothing to do.
        </div>
      </Show>

      <div class="flex-1 overflow-y-auto px-6 py-4 space-y-4">
        <For each={requests()}>
          {(req) => {
            const t = tally(req);
            return (
              <div class="border border-gray-800 rounded-lg bg-gray-900/50 p-4">
                <div class="flex items-start justify-between gap-4">
                  <div class="min-w-0 flex-1">
                    <div class="flex items-center gap-3 mb-1 flex-wrap">
                      <span class="text-cyan-400 font-mono text-sm truncate">
                        {req.branch}
                      </span>
                      <span
                        class={`px-2 py-0.5 rounded text-xs border ${statusBadge(req.status)}`}
                      >
                        {req.status}
                      </span>
                      <span class="text-xs text-gray-500">role {req.role}</span>
                    </div>
                    <div class="text-gray-500 text-xs font-mono truncate">
                      {req.worktree_path}
                    </div>
                    <div class="text-gray-400 text-xs">
                      opened {req.opened_at}
                      <Show when={req.related_workplan}>
                        {" · "}
                        <span class="text-gray-500">workplan</span> {req.related_workplan}
                      </Show>
                      <Show when={req.agent_id}>
                        {" · "}
                        <span class="text-gray-500">agent</span> {req.agent_id}
                      </Show>
                    </div>
                  </div>

                  <Show when={req.status === "open"}>
                    <div class="flex flex-col gap-2 items-end">
                      <button
                        class="px-3 py-1.5 rounded bg-green-700 hover:bg-green-600 text-white text-sm disabled:opacity-50"
                        disabled={busyPath() === req.worktree_path}
                        onClick={() => approve(req.worktree_path)}
                      >
                        Approve
                      </button>
                      <input
                        class="px-2 py-1 rounded bg-gray-950 border border-gray-700 text-xs text-gray-200 w-48"
                        placeholder="rejection reason"
                        value={rejectReason()[req.worktree_path] || ""}
                        onInput={(e) =>
                          setRejectReason({
                            ...rejectReason(),
                            [req.worktree_path]: e.currentTarget.value,
                          })
                        }
                      />
                      <button
                        class="px-3 py-1.5 rounded bg-red-700 hover:bg-red-600 text-white text-sm disabled:opacity-50"
                        disabled={busyPath() === req.worktree_path}
                        onClick={() => reject(req.worktree_path)}
                      >
                        Reject
                      </button>
                    </div>
                  </Show>
                </div>

                <div class="mt-3 grid grid-cols-3 gap-3 text-xs">
                  <div class="rounded bg-gray-950 border border-gray-800 p-2">
                    <div class="text-gray-500 uppercase tracking-wide">tally</div>
                    <div class="mt-1">
                      <span class="text-green-400">+{t.pass}</span>{" "}
                      <span class="text-red-400">−{t.fail}</span>{" "}
                      <span class="text-gray-500">∅{t.abstain}</span>
                    </div>
                    <div class="text-gray-500 mt-1">
                      quorum 2/{REQUIRED_VOTERS.length}
                    </div>
                  </div>

                  <div class="col-span-2 rounded bg-gray-950 border border-gray-800 p-2">
                    <div class="text-gray-500 uppercase tracking-wide">voters</div>
                    <div class="mt-1 space-y-1">
                      <For each={REQUIRED_VOTERS}>
                        {(role) => {
                          const v = req.votes.find((x) => x.voter === role);
                          return (
                            <div class="flex items-center gap-2">
                              <span class="text-gray-300 w-40 truncate">{role}</span>
                              <span class={verdictColor(v?.verdict || "pending")}>
                                {v?.verdict || "pending"}
                              </span>
                              <span class="text-gray-500 truncate">
                                {v?.reason || ""}
                              </span>
                            </div>
                          );
                        }}
                      </For>
                      <Show when={req.votes.some((v) => v.voter === "operator")}>
                        <div class="flex items-center gap-2 pt-1 border-t border-gray-800">
                          <span class="text-yellow-300 w-40 truncate">operator</span>
                          <For each={req.votes.filter((v) => v.voter === "operator")}>
                            {(v) => (
                              <>
                                <span class={verdictColor(v.verdict)}>{v.verdict}</span>
                                <span class="text-gray-500 truncate">{v.reason}</span>
                              </>
                            )}
                          </For>
                        </div>
                      </Show>
                    </div>
                  </div>
                </div>
              </div>
            );
          }}
        </For>
      </div>
    </div>
  );
};

export default MergeGate;
