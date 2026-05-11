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

// Clickable column header that toggles sort key/direction.
interface SortHeaderProps {
  label: string;
  k: SortKey;
  sortKey: () => SortKey;
  sortDir: () => "asc" | "desc";
  onClick: (k: SortKey) => void;
  class?: string;
  title?: string;
}
const SortHeader: Component<SortHeaderProps> = (p) => {
  const active = () => p.sortKey() === p.k;
  return (
    <th
      onClick={() => p.onClick(p.k)}
      title={p.title}
      class={`px-2 py-2 cursor-pointer select-none font-medium hover:text-gray-300 ${p.class ?? "text-left"}`}
    >
      <span class={active() ? "text-cyan-300" : ""}>{p.label}</span>
      <span class="ml-1 text-gray-600">
        {active() ? (p.sortDir() === "asc" ? "↑" : "↓") : ""}
      </span>
    </th>
  );
};

// ── List view ───────────────────────────────────────────────────────────────

type SortKey = "status" | "priority" | "title" | "milestones" | "features" | "budget" | "created";

const PRIORITY_ORDER: Record<string, number> = {
  "P0-BLOCKER": 0,
  high: 1,
  normal: 2,
  "": 3,
};

const STATUS_ORDER: Record<string, number> = {
  in_progress: 0,
  planned: 1,
  blocked: 2,
  done: 3,
  superseded: 4,
  "": 5,
};

const MissionsList: Component = () => {
  const [missions, setMissions] = createSignal<MissionSummary[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [filter, setFilter] = createSignal<"all" | "in_progress" | "planned" | "done">("all");
  const [search, setSearch] = createSignal("");
  const [sortKey, setSortKey] = createSignal<SortKey>("priority");
  const [sortDir, setSortDir] = createSignal<"asc" | "desc">("asc");
  let timer: ReturnType<typeof setInterval> | null = null;

  const toggleSort = (key: SortKey) => {
    if (sortKey() === key) {
      setSortDir(sortDir() === "asc" ? "desc" : "asc");
    } else {
      setSortKey(key);
      setSortDir(key === "milestones" || key === "features" || key === "budget" || key === "created" ? "desc" : "asc");
    }
  };

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
    const key = sortKey();
    const dir = sortDir() === "asc" ? 1 : -1;
    out = [...out].sort((a, b) => {
      let cmp = 0;
      switch (key) {
        case "status":
          cmp = (STATUS_ORDER[a.status ?? ""] ?? 5) - (STATUS_ORDER[b.status ?? ""] ?? 5);
          break;
        case "priority":
          cmp = (PRIORITY_ORDER[a.priority ?? ""] ?? 3) - (PRIORITY_ORDER[b.priority ?? ""] ?? 3);
          break;
        case "title":
          cmp = (a.title || a.feature || a.id).localeCompare(b.title || b.feature || b.id);
          break;
        case "milestones":
          cmp = a.phases - b.phases;
          break;
        case "features":
          cmp = a.tasks - b.tasks;
          break;
        case "budget":
          cmp = budgetEstimate(a) - budgetEstimate(b);
          break;
        case "created":
          cmp = (a.created_at || "").localeCompare(b.created_at || "");
          break;
      }
      return cmp * dir;
    });
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
          <div class="text-xs text-gray-500 mb-2">
            {filtered().length} of {missions().length} missions
            {sortKey() && <span> · sorted by <span class="text-gray-300">{sortKey()}</span> {sortDir() === "asc" ? "↑" : "↓"}</span>}
          </div>
          <div class="bg-gray-900 border border-gray-800 rounded-lg overflow-hidden">
            <table class="w-full text-sm">
              <thead class="bg-gray-950 text-xs uppercase tracking-wide text-gray-500">
                <tr>
                  <SortHeader label="Status" k="status" sortKey={sortKey} sortDir={sortDir} onClick={toggleSort} class="w-24" />
                  <SortHeader label="Pri" k="priority" sortKey={sortKey} sortDir={sortDir} onClick={toggleSort} class="w-24" />
                  <SortHeader label="Mission" k="title" sortKey={sortKey} sortDir={sortDir} onClick={toggleSort} />
                  <SortHeader label="M" k="milestones" sortKey={sortKey} sortDir={sortDir} onClick={toggleSort} class="w-14 text-right" title="Milestones" />
                  <SortHeader label="F" k="features" sortKey={sortKey} sortDir={sortDir} onClick={toggleSort} class="w-14 text-right" title="Features" />
                  <SortHeader label="Budget" k="budget" sortKey={sortKey} sortDir={sortDir} onClick={toggleSort} class="w-20 text-right" title="Factory budget formula: features + 2 × milestones" />
                  <th class="px-2 py-2 text-left w-32 font-medium">ADR</th>
                  <SortHeader label="Created" k="created" sortKey={sortKey} sortDir={sortDir} onClick={toggleSort} class="w-24 text-left" />
                </tr>
              </thead>
              <tbody class="divide-y divide-gray-850">
                <For each={filtered()}>
                  {(m) => (
                    <tr
                      onClick={() => navigate({ page: "mission-detail", missionId: m.id })}
                      class="cursor-pointer hover:bg-gray-850 transition-colors group"
                    >
                      <td class="px-2 py-1.5">
                        <span class={`text-[10px] px-1.5 py-0.5 rounded border ${statusClass(m.status)}`}>
                          {m.status ?? "planned"}
                        </span>
                      </td>
                      <td class="px-2 py-1.5">
                        <Show when={m.priority && m.priority !== "normal"} fallback={
                          <span class="text-[10px] text-gray-600">{m.priority || "—"}</span>
                        }>
                          <span class={`text-[10px] px-1.5 py-0.5 rounded border ${priorityClass(m.priority)}`}>
                            {m.priority}
                          </span>
                        </Show>
                      </td>
                      <td class="px-2 py-1.5 min-w-0">
                        <div class="truncate text-gray-100 group-hover:text-cyan-300">
                          {m.title || m.feature || m.id}
                        </div>
                        <div class="truncate text-[11px] text-gray-600 font-mono">{m.id}</div>
                      </td>
                      <td class="px-2 py-1.5 text-right font-mono text-gray-300">{m.phases}</td>
                      <td class="px-2 py-1.5 text-right font-mono text-gray-300">{m.tasks}</td>
                      <td class="px-2 py-1.5 text-right font-mono text-gray-500">{budgetEstimate(m)}</td>
                      <td class="px-2 py-1.5 text-xs text-gray-500 truncate">
                        {m.related_adrs?.join(", ") || "—"}
                      </td>
                      <td class="px-2 py-1.5 text-xs text-gray-600 whitespace-nowrap">
                        {m.created_at ? m.created_at.split("T")[0] : "—"}
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </div>
        </Show>
      </Show>
    </div>
  );
};

// ── Detail / Ops Console view ───────────────────────────────────────────────

interface EventRow {
  id: number;
  event_type: string;
  tool_name: string | null;
  session_id: string | null;
  created_at: string;
  input_json?: string | null;
  exit_code?: number | null;
  duration_ms?: number | null;
}

// Status dot in features list. Matches Factory's screenshot:
//   ● in flight (orange/cyan), ✓ done (green), × failed (red),
//   ○ planned (gray), − skipped/blocked (gray dim)
const StatusDot: Component<{ status: string }> = (p) => {
  const map: Record<string, { sym: string; cls: string }> = {
    in_progress: { sym: "●", cls: "text-orange-400" },
    done: { sym: "✓", cls: "text-green-400" },
    failed: { sym: "×", cls: "text-red-400" },
    blocked: { sym: "⚠", cls: "text-yellow-400" },
    planned: { sym: "○", cls: "text-gray-600" },
    pending: { sym: "○", cls: "text-gray-600" },
    skipped: { sym: "−", cls: "text-gray-700" },
  };
  const v = () => map[p.status] ?? { sym: "○", cls: "text-gray-600" };
  return <span class={`font-mono text-sm shrink-0 ${v().cls}`}>{v().sym}</span>;
};

// Compact relative-time formatter (Factory uses "<1m ago", "23h ago")
const relTime = (iso: string): string => {
  if (!iso) return "—";
  const t = new Date(iso).getTime();
  if (isNaN(t)) return "—";
  const sec = Math.floor((Date.now() - t) / 1000);
  if (sec < 60) return "<1m ago";
  if (sec < 3600) return `${Math.floor(sec / 60)}m ago`;
  if (sec < 86400) return `${Math.floor(sec / 3600)}h ago`;
  return `${Math.floor(sec / 86400)}d ago`;
};

const MissionDetailView: Component<{ missionId: string }> = (props) => {
  const [mission, setMission] = createSignal<MissionDetail | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [events, setEvents] = createSignal<EventRow[]>([]);
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

  const fetchEvents = async () => {
    try {
      const resp: any = await restClient.get("/api/events?limit=50");
      setEvents(resp.events ?? []);
    } catch (e) {
      // ignore — events are an enhancement, not blocking
    }
  };

  onMount(() => {
    fetchDetail();
    fetchEvents();
    timer = setInterval(() => {
      fetchDetail();
      fetchEvents();
    }, REFRESH_MS);
  });
  onCleanup(() => {
    if (timer) clearInterval(timer);
  });

  // Flat list of features (tasks) with milestone context, for the right pane.
  const features = createMemo<(TaskDetail & { phaseId: string; phaseName: string; phaseIdx: number })[]>(() => {
    const m = mission();
    if (!m?.phases) return [];
    const out: (TaskDetail & { phaseId: string; phaseName: string; phaseIdx: number })[] = [];
    m.phases.forEach((p, idx) => {
      for (const t of p.tasks ?? []) {
        out.push({ ...t, phaseId: p.id, phaseName: p.name, phaseIdx: idx });
      }
    });
    return out;
  });

  const counts = createMemo(() => {
    const fs = features();
    const c = { total: fs.length, done: 0, in_progress: 0, failed: 0, planned: 0, blocked: 0 };
    for (const f of fs) {
      const s = (f.status || "planned") as keyof typeof c;
      if (s in c) (c as any)[s]++;
    }
    return c;
  });

  // The first in_progress task, else the first planned/pending one.
  const currentFeature = createMemo(() => {
    const fs = features();
    return fs.find((f) => f.status === "in_progress")
      ?? fs.find((f) => f.status === "planned" || f.status === "pending")
      ?? null;
  });

  const isRunning = () => counts().in_progress > 0;
  const isComplete = () => counts().total > 0 && counts().done === counts().total;

  const pct = () => {
    const c = counts();
    return c.total > 0 ? Math.round((c.done / c.total) * 100) : 0;
  };

  return (
    <div class="h-screen flex flex-col bg-gray-950 text-gray-100">
      {/* Header bar */}
      <div class="shrink-0 border-b border-gray-800 px-4 py-3">
        <div class="flex items-center justify-between gap-4">
          <div class="flex items-center gap-3 min-w-0">
            <button
              onClick={() => navigate({ page: "missions" })}
              class="text-xs text-gray-500 hover:text-cyan-400 shrink-0"
              title="All missions (Esc)"
            >
              ← All
            </button>
            <span class="text-sm font-semibold text-gray-200 shrink-0">Mission Control</span>
            <Show when={mission()}>
              <span class="text-xs text-gray-500 truncate">
                Mission: <span class="font-mono text-gray-300">{props.missionId}</span>
              </span>
            </Show>
          </div>
          <Show when={mission()}>
            <div class="flex items-center gap-4 text-xs text-gray-500 shrink-0">
              <span>ADR: <span class="text-gray-300">{mission()!.adr}</span></span>
              <span>{counts().total} features</span>
              <span>{mission()!.phases.length} milestones</span>
              <span>budget ≈ {budgetEstimate(mission()!)}</span>
            </div>
          </Show>
        </div>
        <Show when={mission()}>
          <div class="mt-2 text-base font-semibold text-white truncate">{mission()!.feature}</div>
        </Show>
      </div>

      {/* Status / progress bar */}
      <Show when={mission()}>
        <div class="shrink-0 border-b border-gray-800 px-4 py-2 flex items-center gap-3">
          <Show when={isRunning()} fallback={
            <Show when={isComplete()} fallback={
              <span class="text-xs px-2 py-0.5 rounded border bg-gray-800 text-gray-400 border-gray-700">
                ○ {mission()!.status || "planned"}
              </span>
            }>
              <span class="text-xs px-2 py-0.5 rounded border bg-green-900 text-green-200 border-green-700">
                ✓ Complete
              </span>
            </Show>
          }>
            <span class="text-xs px-2 py-0.5 rounded border bg-orange-900 text-orange-200 border-orange-700">
              ● Running
            </span>
          </Show>
          <div class="flex-1 h-2 bg-gray-800 rounded-full overflow-hidden">
            <div
              class={`h-full transition-all ${isRunning() ? "bg-orange-500" : "bg-cyan-600"}`}
              style={{ width: `${pct()}%` }}
            />
          </div>
          <span class="text-xs font-mono text-gray-400 shrink-0 w-32 text-right">
            {counts().done}/{counts().total}
            {counts().failed > 0 && <span class="text-red-400"> ({counts().failed} failed)</span>}
          </span>
        </div>
      </Show>

      <Show when={loading()}>
        <div class="p-6 text-gray-500">Loading mission…</div>
      </Show>
      <Show when={error()}>
        <div class="m-4 bg-red-950 border border-red-800 rounded-lg p-4 text-red-200">
          Failed to load mission: {error()}
        </div>
      </Show>

      <Show when={mission()}>
        <div class="flex-1 min-h-0 grid grid-cols-12 gap-3 p-3">
          {/* Left: Current Feature */}
          <div class="col-span-5 bg-gray-900 border border-orange-900/40 rounded-lg overflow-hidden flex flex-col">
            <div class="px-4 py-2 border-b border-gray-800 text-xs uppercase tracking-wide text-orange-300/80">
              Current Feature
            </div>
            <Show when={currentFeature()} fallback={
              <div class="p-6 text-sm text-gray-500">
                <Show when={isComplete()} fallback="No feature in flight.">
                  Mission complete — all {counts().total} features done.
                </Show>
              </div>
            }>
              {(f) => (
                <div class="p-4 overflow-y-auto flex-1 text-sm">
                  <div class="text-base font-semibold text-gray-100 mb-3">{f().name}</div>
                  <div class="space-y-2 text-xs">
                    <div>
                      <span class="text-gray-500 mr-2">status</span>
                      <StatusDot status={f().status} />
                      <span class="ml-1 text-gray-300">{f().status}</span>
                    </div>
                    <div>
                      <span class="text-gray-500 mr-2">feature id</span>
                      <span class="font-mono text-gray-300">{f().id}</span>
                    </div>
                    <div>
                      <span class="text-gray-500 mr-2">milestone</span>
                      <span class="text-gray-300">M{f().phaseIdx + 1} · {f().phaseName}</span>
                    </div>
                    <Show when={f().layer}>
                      <div>
                        <span class="text-gray-500 mr-2">layer</span>
                        <span class="text-gray-300">{f().layer}</span>
                      </div>
                    </Show>
                    <Show when={f().files && f().files!.length > 0}>
                      <div>
                        <div class="text-gray-500 mb-1">files</div>
                        <ul class="font-mono text-gray-300 space-y-0.5 pl-2">
                          <For each={f().files!}>
                            {(file) => <li class="truncate">{file}</li>}
                          </For>
                        </ul>
                      </div>
                    </Show>
                  </div>
                </div>
              )}
            </Show>
          </div>

          {/* Right: Features list + Progress Log stacked */}
          <div class="col-span-7 flex flex-col gap-3 min-h-0">
            {/* Features list */}
            <div class="bg-gray-900 border border-gray-800 rounded-lg overflow-hidden flex flex-col min-h-0 flex-1">
              <div class="px-4 py-2 border-b border-gray-800 flex items-center justify-between text-xs">
                <span class="uppercase tracking-wide text-gray-500">Features</span>
                <span class="font-mono text-gray-400">
                  {counts().done}/{counts().total}
                  <Show when={counts().failed > 0}>
                    <span class="text-red-400 ml-2">{counts().failed} failed</span>
                  </Show>
                </span>
              </div>
              <div class="overflow-y-auto flex-1 divide-y divide-gray-850">
                <For each={features()}>
                  {(f) => {
                    const isCurrent = () => currentFeature()?.id === f.id;
                    return (
                      <div
                        class={`px-3 py-1.5 flex items-center gap-2 text-sm hover:bg-gray-850 ${
                          isCurrent() ? "bg-orange-950/40" : ""
                        }`}
                      >
                        <StatusDot status={f.status} />
                        <span class={`truncate ${isCurrent() ? "text-orange-200" : "text-gray-300"}`}>
                          {f.name}
                        </span>
                        <span class="ml-auto text-[10px] text-gray-600 font-mono shrink-0">
                          M{f.phaseIdx + 1}
                        </span>
                      </div>
                    );
                  }}
                </For>
              </div>
            </div>

            {/* Progress Log */}
            <div class="bg-gray-900 border border-gray-800 rounded-lg overflow-hidden flex flex-col min-h-0 flex-1">
              <div class="px-4 py-2 border-b border-gray-800 flex items-center justify-between text-xs">
                <span class="uppercase tracking-wide text-gray-500">Progress Log</span>
                <span class="font-mono text-gray-600">{events().length} events</span>
              </div>
              <div class="overflow-y-auto flex-1 divide-y divide-gray-850 font-mono text-xs">
                <Show when={events().length > 0} fallback={
                  <div class="p-3 text-gray-600 italic">No recent events.</div>
                }>
                  <For each={events()}>
                    {(e) => (
                      <div class="px-3 py-1.5 flex items-start gap-2 hover:bg-gray-850">
                        <span class="text-gray-600 w-16 shrink-0">{relTime(e.created_at)}</span>
                        <span class={
                          e.event_type === "PostToolUse" ? "text-green-400 shrink-0" :
                          e.event_type === "PreToolUse" ? "text-cyan-400 shrink-0" :
                          "text-gray-500 shrink-0"
                        }>
                          {e.event_type === "PostToolUse" ? "✓" :
                            e.event_type === "PreToolUse" ? "●" : "·"}
                        </span>
                        <span class="text-gray-400 shrink-0">{e.event_type}</span>
                        <Show when={e.tool_name}>
                          <span class="text-gray-300">{e.tool_name}</span>
                        </Show>
                        <Show when={e.duration_ms !== null && e.duration_ms !== undefined}>
                          <span class="text-gray-600 ml-auto shrink-0">{e.duration_ms}ms</span>
                        </Show>
                      </div>
                    )}
                  </For>
                </Show>
              </div>
            </div>
          </div>
        </div>

        {/* Bottom: raw event stream */}
        <div class="shrink-0 border-t border-gray-800 bg-gray-950 max-h-48 overflow-y-auto">
          <div class="px-4 py-2 border-b border-gray-800 text-xs uppercase tracking-wide text-gray-500 sticky top-0 bg-gray-950">
            Raw stream
          </div>
          <div class="font-mono text-[11px] leading-relaxed p-2">
            <Show when={events().length > 0} fallback={
              <div class="text-gray-700 italic">No raw events yet.</div>
            }>
              <For each={events()}>
                {(e) => {
                  const payload = () => {
                    try {
                      const parsed = JSON.parse(e.input_json ?? "{}");
                      const cmd = parsed.command ?? parsed.description ?? parsed.tool ?? "";
                      return typeof cmd === "string" ? cmd.replace(/\n/g, " ") : JSON.stringify(parsed);
                    } catch {
                      return e.input_json ?? "";
                    }
                  };
                  return (
                    <div class="text-gray-400 truncate">
                      <span class="text-gray-600">→</span>{" "}
                      <span class="text-gray-500">[{e.tool_name ?? e.event_type}]</span>{" "}
                      {payload()}
                    </div>
                  );
                }}
              </For>
            </Show>
          </div>
        </div>

        {/* Footer keyboard hint bar */}
        <div class="shrink-0 border-t border-gray-800 px-4 py-1.5 text-[11px] text-gray-600 flex gap-4">
          <span><span class="text-gray-400">Esc</span> Back</span>
          <span><span class="text-gray-400">F</span> Features</span>
          <span><span class="text-gray-400">L</span> Log</span>
          <span><span class="text-gray-600">P: Pause · M: Models · Tab: Next View (TODO)</span></span>
        </div>
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
