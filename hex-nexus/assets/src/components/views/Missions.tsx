/**
 * Missions.tsx — operator-facing rollup of all workplans as missions.
 *
 * Maps the hex workplan model onto Factory's Missions paradigm:
 *   workplan          → mission
 *   workplan.phases   → milestones
 *   workplan.tasks    → features
 *
 * Reads /api/workplans (on-disk list, summary) and /api/workplan/{id} (detail).
 * No backend changes — pure rollup view on data that already exists.
 *
 * Closes B6 from docs/specs/operator-acceptance-sla.md (plan/milestone view).
 */

import { Component, For, Show, createSignal, onMount, onCleanup, createMemo } from "solid-js";
import { restClient } from "../../services/rest-client";
import { route, navigate } from "../../stores/router";

interface MissionSummary {
  id: string;
  file: string;
  title: string;
  feature?: string;
  created_at: string;
  status?: string;
  priority: string;
  related_adrs: string[];
  phases: number;
  tasks: number;
  progress_pct?: number;
}

interface MissionDetail {
  id: string;
  feature: string;
  description?: string;
  adr: string;
  status: string;
  priority: string;
  phases: PhaseDetail[];
}

interface PhaseDetail {
  id: string;
  name: string;
  status: string;
  tier?: number;
  tasks: TaskDetail[];
}

interface TaskDetail {
  id: string;
  name: string;
  status: string;
  layer?: string;
  files?: string[];
}

const REFRESH_MS = 10000;

const STATUS_COLORS: Record<string, string> = {
  in_progress: "bg-cyan-900 text-cyan-200 border-cyan-700",
  planned: "bg-gray-800 text-gray-300 border-gray-700",
  done: "bg-green-900 text-green-200 border-green-700",
  superseded: "bg-gray-900 text-gray-500 border-gray-800",
  pending: "bg-gray-800 text-gray-400 border-gray-700",
  failed: "bg-red-900 text-red-200 border-red-700",
  blocked: "bg-yellow-900 text-yellow-200 border-yellow-700",
};

const PRIORITY_COLORS: Record<string, string> = {
  "P0-BLOCKER": "bg-red-950 text-red-300 border-red-800",
  high: "bg-orange-900 text-orange-200 border-orange-700",
  normal: "bg-gray-800 text-gray-400 border-gray-700",
};

const statusClass = (s: string | undefined): string =>
  STATUS_COLORS[s ?? ""] ?? "bg-gray-800 text-gray-400 border-gray-700";

const priorityClass = (p: string | undefined): string =>
  PRIORITY_COLORS[p ?? ""] ?? "bg-gray-800 text-gray-400 border-gray-700";

// Factory's budget formula: total runs ≈ #features + 2 × #milestones.
// List summary has phases:number + tasks:number.
// Detail has phases:PhaseDetail[] (tasks live inside each phase).
const budgetEstimate = (m: MissionSummary | MissionDetail): number => {
  if (Array.isArray((m as MissionDetail).phases)) {
    const detail = m as MissionDetail;
    const features = detail.phases.reduce((n, p) => n + (p.tasks?.length ?? 0), 0);
    return features + 2 * detail.phases.length;
  }
  const summary = m as MissionSummary;
  return (summary.tasks ?? 0) + 2 * (summary.phases ?? 0);
};

// ── List view ───────────────────────────────────────────────────────────────

const MissionsList: Component = () => {
  const [missions, setMissions] = createSignal<MissionSummary[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [filter, setFilter] = createSignal<"all" | "in_progress" | "planned" | "done">("all");
  const [search, setSearch] = createSignal("");
  let timer: ReturnType<typeof setInterval> | null = null;

  const fetchMissions = async () => {
    try {
      const resp: any = await restClient.get("/api/workplans");
      const items: MissionSummary[] = resp.workplans ?? [];
      // Compute progress: tasks with status=done / total tasks. The summary
      // endpoint doesn't carry per-task status, so progress=undefined here
      // until the detail endpoint is opened.
      setMissions(items);
    } catch (e) {
      console.error("missions: fetch failed", e);
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    fetchMissions();
    timer = setInterval(fetchMissions, REFRESH_MS);
  });
  onCleanup(() => {
    if (timer) clearInterval(timer);
  });

  const filtered = createMemo(() => {
    let out = missions();
    const s = search().toLowerCase();
    if (s) {
      out = out.filter((m) =>
        (m.title?.toLowerCase().includes(s) ?? false) ||
        (m.feature?.toLowerCase().includes(s) ?? false) ||
        m.id.toLowerCase().includes(s),
      );
    }
    if (filter() !== "all") {
      out = out.filter((m) => (m.status ?? "planned") === filter());
    }
    return out;
  });

  const counts = createMemo(() => {
    const c = { all: missions().length, in_progress: 0, planned: 0, done: 0 };
    for (const m of missions()) {
      const s = (m.status ?? "planned") as keyof typeof c;
      if (s in c) c[s]++;
    }
    return c;
  });

  return (
    <div class="p-6 max-w-7xl mx-auto">
      <div class="mb-6">
        <h1 class="text-2xl font-bold text-white mb-1">Missions</h1>
        <p class="text-sm text-gray-400">
          Each mission = one workplan. Milestones = phases. Features = tasks.
          Budget estimate uses Factory's formula:{" "}
          <code class="text-cyan-300">runs ≈ #features + 2 × #milestones</code>.
        </p>
      </div>

      {/* Filter bar */}
      <div class="flex flex-wrap items-center gap-2 mb-4">
        <For each={[
          ["all", "All"],
          ["in_progress", "In flight"],
          ["planned", "Planned"],
          ["done", "Done"],
        ] as const}>
          {([key, label]) => (
            <button
              onClick={() => setFilter(key)}
              class={`px-3 py-1.5 text-sm rounded-md border transition-colors ${
                filter() === key
                  ? "bg-cyan-900 text-cyan-100 border-cyan-700"
                  : "bg-gray-900 text-gray-400 border-gray-800 hover:border-gray-600"
              }`}
            >
              {label} <span class="ml-1 text-xs text-gray-500">({counts()[key]})</span>
            </button>
          )}
        </For>
        <input
          type="text"
          placeholder="Filter by title, feature, or id…"
          value={search()}
          onInput={(e) => setSearch(e.currentTarget.value)}
          class="flex-1 min-w-[200px] bg-gray-900 border border-gray-800 rounded-md px-3 py-1.5 text-sm text-gray-200 placeholder-gray-600 focus:border-cyan-700 focus:outline-none"
        />
      </div>

      <Show when={!loading()} fallback={<div class="text-gray-500">Loading missions…</div>}>
        <Show when={filtered().length > 0} fallback={
          <div class="bg-gray-900 border border-gray-800 rounded-lg p-8 text-center text-gray-500">
            No missions match the current filter.
          </div>
        }>
          <div class="space-y-2">
            <For each={filtered()}>
              {(m) => (
                <button
                  onClick={() => navigate({ page: "mission-detail", missionId: m.id })}
                  class="w-full text-left bg-gray-900 border border-gray-800 hover:border-cyan-800 rounded-lg p-4 transition-colors group"
                >
                  <div class="flex items-start justify-between gap-4">
                    <div class="flex-1 min-w-0">
                      <div class="flex items-center gap-2 mb-1 flex-wrap">
                        <span class={`text-xs px-2 py-0.5 rounded border ${statusClass(m.status)}`}>
                          {m.status ?? "planned"}
                        </span>
                        <Show when={m.priority}>
                          <span class={`text-xs px-2 py-0.5 rounded border ${priorityClass(m.priority)}`}>
                            {m.priority}
                          </span>
                        </Show>
                        <Show when={m.related_adrs?.length > 0}>
                          <span class="text-xs text-gray-500">{m.related_adrs.join(", ")}</span>
                        </Show>
                      </div>
                      <h3 class="text-base font-semibold text-gray-100 group-hover:text-cyan-300 truncate">
                        {m.title || m.feature || m.id}
                      </h3>
                      <div class="text-xs text-gray-500 mt-1 font-mono truncate">{m.id}</div>
                    </div>
                    <div class="flex flex-col items-end text-xs text-gray-400 shrink-0">
                      <div><span class="text-gray-200 font-mono">{m.phases}</span> milestones</div>
                      <div><span class="text-gray-200 font-mono">{m.tasks}</span> features</div>
                      <div class="text-gray-600 mt-1">budget ≈ {budgetEstimate(m)} runs</div>
                    </div>
                  </div>
                </button>
              )}
            </For>
          </div>
        </Show>
      </Show>
    </div>
  );
};

// ── Detail view ─────────────────────────────────────────────────────────────

const MissionDetailView: Component<{ missionId: string }> = (props) => {
  const [mission, setMission] = createSignal<MissionDetail | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  let timer: ReturnType<typeof setInterval> | null = null;

  const fetchDetail = async () => {
    try {
      const resp: any = await restClient.get(`/api/workplans/${encodeURIComponent(props.missionId)}`);
      const wp = resp.workplan ?? resp.data ?? resp;
      setMission(wp);
      setError(null);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    fetchDetail();
    timer = setInterval(fetchDetail, REFRESH_MS);
  });
  onCleanup(() => {
    if (timer) clearInterval(timer);
  });

  const progress = createMemo(() => {
    const m = mission();
    if (!m?.phases) return { done: 0, total: 0, pct: 0 };
    let done = 0;
    let total = 0;
    for (const p of m.phases) {
      for (const t of p.tasks ?? []) {
        total++;
        if (t.status === "done") done++;
      }
    }
    return { done, total, pct: total > 0 ? Math.round((done / total) * 100) : 0 };
  });

  return (
    <div class="p-6 max-w-6xl mx-auto">
      <button
        onClick={() => navigate({ page: "missions" })}
        class="text-sm text-gray-500 hover:text-cyan-400 mb-4"
      >
        ← All missions
      </button>

      <Show when={loading()}>
        <div class="text-gray-500">Loading mission…</div>
      </Show>
      <Show when={error()}>
        <div class="bg-red-950 border border-red-800 rounded-lg p-4 text-red-200">
          Failed to load mission: {error()}
        </div>
      </Show>
      <Show when={mission()}>
        {(m) => (
          <>
            <div class="mb-6">
              <div class="flex items-center gap-2 mb-2 flex-wrap">
                <span class={`text-xs px-2 py-0.5 rounded border ${statusClass(m().status)}`}>
                  {m().status}
                </span>
                <span class={`text-xs px-2 py-0.5 rounded border ${priorityClass(m().priority)}`}>
                  {m().priority || "normal"}
                </span>
                <span class="text-xs text-gray-500">{m().adr}</span>
              </div>
              <h1 class="text-2xl font-bold text-white">{m().feature}</h1>
              <div class="text-xs text-gray-600 mt-1 font-mono">{m().id}</div>
              <Show when={m().description}>
                <p class="mt-2 text-sm text-gray-400">{m().description}</p>
              </Show>
            </div>

            {/* Progress bar */}
            <div class="bg-gray-900 border border-gray-800 rounded-lg p-4 mb-6">
              <div class="flex items-center justify-between mb-2">
                <span class="text-sm text-gray-300">
                  {progress().done} of {progress().total} features done · {m().phases.length} milestones
                </span>
                <span class="text-sm font-mono text-cyan-300">{progress().pct}%</span>
              </div>
              <div class="h-2 bg-gray-800 rounded-full overflow-hidden">
                <div
                  class="h-full bg-cyan-600 transition-all"
                  style={{ width: `${progress().pct}%` }}
                />
              </div>
              <div class="text-xs text-gray-600 mt-2">
                budget estimate ≈ {budgetEstimate(m())} worker runs
                {" "}({progress().total} features + 2 × {m().phases.length} milestones, Factory formula)
              </div>
            </div>

            {/* Milestones */}
            <div class="space-y-4">
              <For each={m().phases}>
                {(phase, phaseIdx) => {
                  const phaseProgress = () => {
                    const done = (phase.tasks ?? []).filter((t) => t.status === "done").length;
                    const total = (phase.tasks ?? []).length;
                    return { done, total, pct: total > 0 ? Math.round((done / total) * 100) : 0 };
                  };
                  return (
                    <div class="bg-gray-900 border border-gray-800 rounded-lg overflow-hidden">
                      <div class="px-4 py-3 border-b border-gray-800 flex items-center justify-between">
                        <div class="flex items-center gap-2">
                          <span class="text-xs text-gray-600 font-mono">M{phaseIdx() + 1}</span>
                          <span class={`text-xs px-2 py-0.5 rounded border ${statusClass(phase.status)}`}>
                            {phase.status}
                          </span>
                          <h3 class="text-sm font-semibold text-gray-200">{phase.name}</h3>
                          <Show when={phase.tier !== undefined}>
                            <span class="text-xs text-gray-600">tier {phase.tier}</span>
                          </Show>
                        </div>
                        <span class="text-xs text-gray-500 font-mono">
                          {phaseProgress().done}/{phaseProgress().total}
                        </span>
                      </div>
                      <div class="divide-y divide-gray-800">
                        <For each={phase.tasks ?? []}>
                          {(task) => (
                            <div class="px-4 py-2.5 flex items-start gap-3 hover:bg-gray-850">
                              <span class={`text-[10px] mt-0.5 px-1.5 py-0.5 rounded border shrink-0 ${statusClass(task.status)}`}>
                                {task.status}
                              </span>
                              <div class="flex-1 min-w-0">
                                <div class="text-sm text-gray-200">
                                  <span class="text-gray-600 font-mono mr-2">{task.id}</span>
                                  {task.name}
                                </div>
                                <Show when={task.layer || (task.files && task.files.length > 0)}>
                                  <div class="text-xs text-gray-600 mt-1 flex flex-wrap gap-x-3 gap-y-0.5">
                                    <Show when={task.layer}>
                                      <span>layer: <span class="text-gray-500">{task.layer}</span></span>
                                    </Show>
                                    <Show when={task.files && task.files.length > 0}>
                                      <For each={task.files}>
                                        {(f) => <span class="text-gray-500 font-mono">{f}</span>}
                                      </For>
                                    </Show>
                                  </div>
                                </Show>
                              </div>
                            </div>
                          )}
                        </For>
                      </div>
                    </div>
                  );
                }}
              </For>
            </div>
          </>
        )}
      </Show>
    </div>
  );
};

// ── Router-aware shell ──────────────────────────────────────────────────────

const Missions: Component = () => {
  return (
    <Show when={route().page === "mission-detail"} fallback={<MissionsList />}>
      <MissionDetailView missionId={(route() as any).missionId} />
    </Show>
  );
};

export default Missions;
