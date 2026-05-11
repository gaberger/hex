/**
 * Missions.tsx — operator-facing mission rollup.
 *
 * Paradigm (corrected 2026-05-11):
 *   ADR             = mission (the strategic decision)
 *   workplans       = implementation plans that reference this ADR
 *   workplan.phases = milestones inside one workplan
 *   workplan.tasks  = features inside a milestone
 *
 * Catalog (#/missions): table of all ADRs with rollup of workplan count,
 * feature count, and progress across all workplans that reference each ADR.
 *
 * Ops console (#/missions/{adr-id}): the ADR as a running mission, with all
 * its workplans inline, status dots per feature, progress log, raw stream,
 * action buttons (pause/resume/mark-complete/re-assess), and an orchestrator
 * chat panel that DMs a chosen persona.
 *
 * Closes B6 from docs/specs/operator-acceptance-sla.md (plan/milestone view).
 */

import {
  Component, For, Show, createSignal, onMount, onCleanup, createMemo,
} from "solid-js";
import { restClient } from "../../services/rest-client";
import { route, navigate } from "../../stores/router";

// ── Data shapes ─────────────────────────────────────────────────────────────

interface AdrSummary {
  id: string;
  filename: string;
  title: string;
  status: string;
  date: string;
}

interface WorkplanSummary {
  id: string;
  file: string;
  title: string;
  feature: string;
  adr: string;
  adrs: string[];
  status: string;
  priority: string;
  created_at: string;
  phases: number;
  tasks: number;
}

interface WorkplanDetail {
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

interface EventRow {
  id: number;
  event_type: string;
  tool_name: string | null;
  session_id: string | null;
  created_at: string;
  input_json?: string | null;
  duration_ms?: number | null;
}

interface OrgMessage {
  id: number;
  from: string;
  to: string;
  content: string;
  timestamp: string;
}

interface ToastMsg {
  text: string;
  tone: "ok" | "err" | "info";
}

// Flattened feature with milestone + workplan context, for rendering rows
// in the Features list of the ops console.
interface FeatureRow extends TaskDetail {
  workplanId: string;
  workplanTitle: string;
  phaseId: string;
  phaseName: string;
  phaseIdx: number;
}

// ── Constants ───────────────────────────────────────────────────────────────

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

const ADR_STATUS_COLOR = (status: string): string => {
  const s = (status || "").toLowerCase();
  if (s.startsWith("accepted")) return "bg-green-900 text-green-200 border-green-700";
  if (s.startsWith("proposed")) return "bg-cyan-900 text-cyan-200 border-cyan-700";
  if (s.startsWith("superseded")) return "bg-gray-900 text-gray-500 border-gray-800";
  if (s.startsWith("rejected")) return "bg-red-900 text-red-200 border-red-700";
  if (s.startsWith("deprecated")) return "bg-yellow-900 text-yellow-200 border-yellow-700";
  return "bg-gray-800 text-gray-400 border-gray-700";
};

const statusClass = (s: string | undefined): string =>
  STATUS_COLORS[s ?? ""] ?? "bg-gray-800 text-gray-400 border-gray-700";

const priorityClass = (p: string | undefined): string =>
  PRIORITY_COLORS[p ?? ""] ?? "bg-gray-800 text-gray-400 border-gray-700";

// Status dot, Factory paradigm:
//   ● in flight (orange), ✓ done (green), × failed (red),
//   ○ planned (gray), ⚠ blocked (yellow), − skipped (gray dim)
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

// ── List view (Catalog of ADRs as Missions) ─────────────────────────────────

interface AdrRow extends AdrSummary {
  workplanCount: number;
  totalFeatures: number;
  doneFeatures: number;
  inFlight: number;
  pct: number;
  priority: string;       // highest priority across workplans
  workplanStatus: string; // worst-of-all rollup
}

type SortKey = "status" | "id" | "title" | "workplans" | "features" | "progress" | "date";

const STATUS_ORDER: Record<string, number> = {
  proposed: 0,
  accepted: 1,
  deprecated: 2,
  rejected: 3,
  superseded: 4,
  "": 5,
};

const SortHeader: Component<{
  label: string;
  k: SortKey;
  sortKey: () => SortKey;
  sortDir: () => "asc" | "desc";
  onClick: (k: SortKey) => void;
  class?: string;
  title?: string;
}> = (p) => {
  const active = () => p.sortKey() === p.k;
  return (
    <th
      onClick={() => p.onClick(p.k)}
      title={p.title}
      class={`px-2 py-2 cursor-pointer select-none font-medium hover:text-gray-300 ${p.class ?? "text-left"}`}
    >
      <span class={active() ? "text-cyan-300" : ""}>{p.label}</span>
      <span class="ml-1 text-gray-600">{active() ? (p.sortDir() === "asc" ? "↑" : "↓") : ""}</span>
    </th>
  );
};

const MissionsList: Component = () => {
  const [adrs, setAdrs] = createSignal<AdrSummary[]>([]);
  const [workplans, setWorkplans] = createSignal<WorkplanSummary[]>([]);
  // Optional: per-workplan detail to compute task-done counts. Lazy-fetched
  // for visible rows so 81 workplans don't all load eagerly.
  const [wpDetails, setWpDetails] = createSignal<Map<string, WorkplanDetail>>(new Map());
  const [loading, setLoading] = createSignal(true);
  const [filter, setFilter] = createSignal<"all" | "in_flight" | "accepted" | "proposed" | "stale">("all");
  const [search, setSearch] = createSignal("");
  const [sortKey, setSortKey] = createSignal<SortKey>("id");
  const [sortDir, setSortDir] = createSignal<"asc" | "desc">("desc");
  let timer: ReturnType<typeof setInterval> | null = null;

  const toggleSort = (key: SortKey) => {
    if (sortKey() === key) {
      setSortDir(sortDir() === "asc" ? "desc" : "asc");
    } else {
      setSortKey(key);
      setSortDir(key === "id" || key === "date" || key === "workplans" || key === "features" || key === "progress" ? "desc" : "asc");
    }
  };

  const fetchAdrs = async () => {
    try {
      const resp: any = await restClient.get("/api/adrs");
      const list: AdrSummary[] = Array.isArray(resp) ? resp : (resp.adrs ?? []);
      setAdrs(list);
    } catch (e) {
      console.error("missions: ADR fetch failed", e);
    }
  };

  const fetchWorkplans = async () => {
    try {
      const resp: any = await restClient.get("/api/workplans");
      setWorkplans(resp.workplans ?? []);
    } catch (e) {
      console.error("missions: workplan fetch failed", e);
    }
  };

  const lazyFetchDetail = async (wpId: string) => {
    if (wpDetails().has(wpId)) return;
    try {
      const resp: any = await restClient.get(`/api/workplans/${encodeURIComponent(wpId)}`);
      const wp = resp.workplan ?? resp;
      setWpDetails((m) => new Map(m).set(wpId, wp));
    } catch {
      // swallow — rollup just won't show task-level progress
    }
  };

  onMount(async () => {
    await Promise.all([fetchAdrs(), fetchWorkplans()]);
    setLoading(false);
    timer = setInterval(() => {
      fetchAdrs();
      fetchWorkplans();
    }, REFRESH_MS);
  });
  onCleanup(() => {
    if (timer) clearInterval(timer);
  });

  // Workplans grouped by ADR id. Multiple workplans can reference one ADR.
  const wpByAdr = createMemo<Map<string, WorkplanSummary[]>>(() => {
    const out = new Map<string, WorkplanSummary[]>();
    for (const wp of workplans()) {
      const adr = (wp.adr || "").trim();
      if (!adr) continue;
      const arr = out.get(adr) ?? [];
      arr.push(wp);
      out.set(adr, arr);
    }
    return out;
  });

  const rows = createMemo<AdrRow[]>(() => {
    const map = wpByAdr();
    const det = wpDetails();
    return adrs().map((a) => {
      const wps = map.get(a.id) ?? [];
      let totalFeatures = 0;
      let doneFeatures = 0;
      let inFlight = 0;
      let highestPri = "";
      let worstStatus = "";
      for (const wp of wps) {
        totalFeatures += wp.tasks;
        if ((wp.priority === "P0-BLOCKER") || (highestPri !== "P0-BLOCKER" && wp.priority === "high")) {
          highestPri = wp.priority;
        }
        if (!highestPri && wp.priority) highestPri = wp.priority;
        if (wp.status === "in_progress") {
          worstStatus = "in_progress";
        } else if (wp.status && !worstStatus) {
          worstStatus = wp.status;
        }
        const d = det.get(wp.id);
        if (d?.phases) {
          for (const ph of d.phases) {
            for (const t of ph.tasks ?? []) {
              if (t.status === "done") doneFeatures++;
              if (t.status === "in_progress") inFlight++;
            }
          }
        }
      }
      const pct = totalFeatures > 0 ? Math.round((doneFeatures / totalFeatures) * 100) : 0;
      return {
        ...a,
        workplanCount: wps.length,
        totalFeatures,
        doneFeatures,
        inFlight,
        pct,
        priority: highestPri,
        workplanStatus: worstStatus,
      };
    });
  });

  const filtered = createMemo<AdrRow[]>(() => {
    let out = rows();
    const s = search().toLowerCase();
    if (s) {
      out = out.filter((r) =>
        r.id.toLowerCase().includes(s) ||
        (r.title?.toLowerCase().includes(s) ?? false) ||
        (r.filename?.toLowerCase().includes(s) ?? false),
      );
    }
    const f = filter();
    if (f === "in_flight") out = out.filter((r) => r.inFlight > 0 || r.workplanStatus === "in_progress");
    else if (f === "accepted") out = out.filter((r) => (r.status || "").toLowerCase().startsWith("accepted"));
    else if (f === "proposed") out = out.filter((r) => (r.status || "").toLowerCase().startsWith("proposed"));
    else if (f === "stale") out = out.filter((r) => r.workplanCount === 0);

    const key = sortKey();
    const dir = sortDir() === "asc" ? 1 : -1;
    out = [...out].sort((a, b) => {
      let cmp = 0;
      switch (key) {
        case "status":
          cmp = (STATUS_ORDER[(a.status || "").toLowerCase().split(" ")[0]] ?? 5)
              - (STATUS_ORDER[(b.status || "").toLowerCase().split(" ")[0]] ?? 5);
          break;
        case "id":
          cmp = a.id.localeCompare(b.id);
          break;
        case "title":
          cmp = (a.title || "").localeCompare(b.title || "");
          break;
        case "workplans":
          cmp = a.workplanCount - b.workplanCount;
          break;
        case "features":
          cmp = a.totalFeatures - b.totalFeatures;
          break;
        case "progress":
          cmp = a.pct - b.pct;
          break;
        case "date":
          cmp = (a.date || "").localeCompare(b.date || "");
          break;
      }
      return cmp * dir;
    });
    return out;
  });

  // Trigger lazy detail fetch for filtered rows (so the progress column has real
  // numbers). Capped at 30 per refresh so we don't hammer.
  createMemo(() => {
    let n = 0;
    for (const r of filtered()) {
      for (const wp of (wpByAdr().get(r.id) ?? [])) {
        if (!wpDetails().has(wp.id)) {
          lazyFetchDetail(wp.id);
          n++;
          if (n > 30) return;
        }
      }
    }
  });

  const counts = createMemo(() => {
    const r = rows();
    return {
      all: r.length,
      in_flight: r.filter((x) => x.inFlight > 0 || x.workplanStatus === "in_progress").length,
      accepted: r.filter((x) => (x.status || "").toLowerCase().startsWith("accepted")).length,
      proposed: r.filter((x) => (x.status || "").toLowerCase().startsWith("proposed")).length,
      stale: r.filter((x) => x.workplanCount === 0).length,
    };
  });

  return (
    <div class="p-6 max-w-7xl mx-auto">
      <div class="mb-6">
        <h1 class="text-2xl font-bold text-white mb-1">Missions</h1>
        <p class="text-sm text-gray-400">
          Each mission = one ADR. Workplans + their phases + tasks roll up under the ADR.
          Click a row to open the ops console.
        </p>
      </div>

      <div class="flex flex-wrap items-center gap-2 mb-4">
        <For each={[
          ["all", "All"],
          ["in_flight", "In flight"],
          ["accepted", "Accepted"],
          ["proposed", "Proposed"],
          ["stale", "No workplan"],
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
              {label}
              <span class="ml-1 text-xs text-gray-500">({counts()[key]})</span>
            </button>
          )}
        </For>
        <input
          type="text"
          placeholder="Filter by ADR id, title, or filename…"
          value={search()}
          onInput={(e) => setSearch(e.currentTarget.value)}
          class="flex-1 min-w-[200px] bg-gray-900 border border-gray-800 rounded-md px-3 py-1.5 text-sm text-gray-200 placeholder-gray-600 focus:border-cyan-700 focus:outline-none"
        />
      </div>

      <Show when={!loading()} fallback={<div class="text-gray-500">Loading missions…</div>}>
        <Show when={filtered().length > 0} fallback={
          <div class="bg-gray-900 border border-gray-800 rounded-lg p-8 text-center text-gray-500">
            No ADRs match the current filter.
          </div>
        }>
          <div class="text-xs text-gray-500 mb-2">
            {filtered().length} of {rows().length} missions
            <span> · sorted by <span class="text-gray-300">{sortKey()}</span> {sortDir() === "asc" ? "↑" : "↓"}</span>
          </div>
          <div class="bg-gray-900 border border-gray-800 rounded-lg overflow-hidden">
            <table class="w-full text-sm">
              <thead class="bg-gray-950 text-xs uppercase tracking-wide text-gray-500">
                <tr>
                  <SortHeader label="Status" k="status" sortKey={sortKey} sortDir={sortDir} onClick={toggleSort} class="w-32" />
                  <SortHeader label="ADR" k="id" sortKey={sortKey} sortDir={sortDir} onClick={toggleSort} class="w-32 font-mono" />
                  <SortHeader label="Title" k="title" sortKey={sortKey} sortDir={sortDir} onClick={toggleSort} />
                  <SortHeader label="Workplans" k="workplans" sortKey={sortKey} sortDir={sortDir} onClick={toggleSort} class="w-20 text-right" />
                  <SortHeader label="Features" k="features" sortKey={sortKey} sortDir={sortDir} onClick={toggleSort} class="w-20 text-right" />
                  <SortHeader label="Progress" k="progress" sortKey={sortKey} sortDir={sortDir} onClick={toggleSort} class="w-32 text-right" />
                </tr>
              </thead>
              <tbody class="divide-y divide-gray-850">
                <For each={filtered()}>
                  {(r) => (
                    <tr
                      onClick={() => navigate({ page: "mission-detail", missionId: r.id })}
                      class="cursor-pointer hover:bg-gray-850 transition-colors group"
                    >
                      <td class="px-2 py-1.5">
                        <Show when={r.inFlight > 0}>
                          <span class="text-[10px] px-1.5 py-0.5 rounded border bg-orange-900 text-orange-200 border-orange-700 mr-1">
                            ● {r.inFlight}
                          </span>
                        </Show>
                        <span class={`text-[10px] px-1.5 py-0.5 rounded border ${ADR_STATUS_COLOR(r.status)}`}>
                          {(r.status || "—").split(" ")[0]}
                        </span>
                      </td>
                      <td class="px-2 py-1.5 font-mono text-xs text-gray-400">{r.id}</td>
                      <td class="px-2 py-1.5 min-w-0">
                        <div class="truncate text-gray-100 group-hover:text-cyan-300">{r.title || r.filename}</div>
                        <Show when={r.priority && r.priority !== "normal"}>
                          <span class={`text-[10px] px-1.5 py-0.5 rounded border ${priorityClass(r.priority)} mt-0.5 inline-block`}>
                            {r.priority}
                          </span>
                        </Show>
                      </td>
                      <td class="px-2 py-1.5 text-right font-mono text-gray-300">{r.workplanCount}</td>
                      <td class="px-2 py-1.5 text-right font-mono text-gray-300">
                        {r.totalFeatures > 0 ? `${r.doneFeatures}/${r.totalFeatures}` : "—"}
                      </td>
                      <td class="px-2 py-1.5 w-32">
                        <Show when={r.totalFeatures > 0} fallback={<span class="text-xs text-gray-700">—</span>}>
                          <div class="flex items-center gap-2">
                            <div class="flex-1 h-1.5 bg-gray-800 rounded-full overflow-hidden">
                              <div
                                class={r.inFlight > 0 ? "h-full bg-orange-500" : "h-full bg-cyan-600"}
                                style={{ width: `${r.pct}%` }}
                              />
                            </div>
                            <span class="text-[11px] font-mono text-gray-500 w-9 text-right">{r.pct}%</span>
                          </div>
                        </Show>
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

// ── Ops Console (one ADR / Mission detail) ──────────────────────────────────

const MissionDetailView: Component<{ missionId: string }> = (props) => {
  const [adr, setAdr] = createSignal<{ id: string; title: string; status: string; date: string; body?: string } | null>(null);
  const [workplans, setWorkplans] = createSignal<WorkplanDetail[]>([]);
  const [events, setEvents] = createSignal<EventRow[]>([]);
  const [chatMessages, setChatMessages] = createSignal<OrgMessage[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [selectedFeatureId, setSelectedFeatureId] = createSignal<string | null>(null);
  const [actionInFlight, setActionInFlight] = createSignal<string | null>(null);
  const [toast, setToast] = createSignal<ToastMsg | null>(null);
  const [chatInput, setChatInput] = createSignal("");
  const [chatPersona, setChatPersona] = createSignal<"engineering-lead" | "cto" | "cpo" | "coo">("engineering-lead");
  const [chatSending, setChatSending] = createSignal(false);
  const [showHeartbeats, setShowHeartbeats] = createSignal(false);
  let timer: ReturnType<typeof setInterval> | null = null;
  let currentPaneRef: HTMLDivElement | undefined;
  let featuresPaneRef: HTMLDivElement | undefined;
  let logPaneRef: HTMLDivElement | undefined;
  let chatPaneRef: HTMLDivElement | undefined;

  const showToast = (text: string, tone: ToastMsg["tone"] = "info") => {
    setToast({ text, tone });
    setTimeout(() => setToast(null), 3500);
  };

  const flashPane = (ref: HTMLDivElement | undefined) => {
    if (!ref) return;
    ref.scrollIntoView({ behavior: "smooth", block: "nearest" });
    ref.classList.add("ring-2", "ring-cyan-500/40");
    setTimeout(() => ref.classList.remove("ring-2", "ring-cyan-500/40"), 800);
  };

  const fetchAdr = async () => {
    try {
      const resp: any = await restClient.get(`/api/adrs/${encodeURIComponent(props.missionId)}`);
      // Response can be {id,title,status,body} or {data:{...}}
      const a = resp.data ?? resp;
      setAdr({
        id: a.id ?? props.missionId,
        title: a.title ?? "",
        status: a.status ?? "",
        date: a.date ?? "",
        body: a.body ?? a.content ?? "",
      });
      setError(null);
    } catch (e: any) {
      setError(String(e?.message ?? e));
    }
  };

  const fetchWorkplans = async () => {
    try {
      const list: any = await restClient.get("/api/workplans");
      const summaries: WorkplanSummary[] = list.workplans ?? [];
      const matching = summaries.filter((w) => (w.adr || "").trim() === props.missionId);
      // Fetch each detail in parallel
      const details = await Promise.all(
        matching.map(async (m) => {
          try {
            const r: any = await restClient.get(`/api/workplans/${encodeURIComponent(m.id)}`);
            return (r.workplan ?? r) as WorkplanDetail;
          } catch {
            return null;
          }
        }),
      );
      setWorkplans(details.filter(Boolean) as WorkplanDetail[]);
    } catch (e) {
      console.error("missions detail: workplan fetch failed", e);
    } finally {
      setLoading(false);
    }
  };

  // Heartbeat tools that fire every few seconds and clog the log with noise.
  // They aren't mission-relevant — we hide them unless the operator opts in.
  const HEARTBEAT_TOOLS = new Set([
    "brain_tick", "adr_doctor_tick", "improver_tick",
    "supervisor_tick", "persona_tick", "session_heartbeat",
    "resource_observation", "config_sync",
  ]);

  const fetchEvents = async () => {
    try {
      // Fetch a wider window so post-filter we still have ~50 meaningful rows.
      const r: any = await restClient.get("/api/events?limit=200");
      const raw: EventRow[] = r.events ?? [];
      const filtered = showHeartbeats()
        ? raw
        : raw.filter((e) => !HEARTBEAT_TOOLS.has(e.tool_name ?? ""));
      setEvents(filtered.slice(0, 80));
    } catch {}
  };

  const fetchChat = async () => {
    try {
      const r: any = await restClient.get("/api/org/messages?limit=80");
      const all: OrgMessage[] = r.messages ?? [];
      const idLow = props.missionId.toLowerCase();
      const filtered = all.filter((m) => {
        const c = (m.content || "").toLowerCase();
        return c.includes(idLow);
      });
      filtered.sort((a, b) => a.id - b.id);
      setChatMessages(filtered.slice(-30));
    } catch {}
  };

  // Flat feature list across all workplans for this ADR.
  const features = createMemo<FeatureRow[]>(() => {
    const out: FeatureRow[] = [];
    for (const wp of workplans()) {
      wp.phases?.forEach((p, idx) => {
        for (const t of p.tasks ?? []) {
          out.push({
            ...t,
            workplanId: wp.id,
            workplanTitle: wp.feature || wp.id,
            phaseId: p.id,
            phaseName: p.name,
            phaseIdx: idx,
          });
        }
      });
    }
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

  const currentFeature = createMemo<FeatureRow | null>(() => {
    const fs = features();
    const sel = selectedFeatureId();
    if (sel) {
      const m = fs.find((f) => `${f.workplanId}:${f.id}` === sel);
      if (m) return m;
    }
    return fs.find((f) => f.status === "in_progress")
      ?? fs.find((f) => f.status === "planned" || f.status === "pending")
      // Fallback: first not-done feature so the operator always sees something
      ?? fs.find((f) => f.status !== "done")
      ?? fs[0]
      ?? null;
  });

  const isRunning = () => counts().in_progress > 0;
  const isComplete = () => counts().total > 0 && counts().done === counts().total;
  const pct = () => counts().total > 0 ? Math.round((counts().done / counts().total) * 100) : 0;

  // Operator actions. pause/resume call the workplan API (we pause/resume
  // every workplan that references this ADR). mark-current-complete and
  // re-assess are B7-pending; we send them as DMs so operator intent is
  // captured even before the supervisor verb exists.
  const doAction = async (verb: "pause" | "resume" | "mark-current-complete" | "re-assess") => {
    if (actionInFlight()) return;
    setActionInFlight(verb);
    try {
      if (verb === "pause" || verb === "resume") {
        const wpIds = workplans().map((w) => w.id);
        if (wpIds.length === 0) {
          showToast(`No workplans under ${props.missionId} to ${verb}`, "info");
        } else {
          await Promise.all(wpIds.map((id) =>
            restClient.post(`/api/workplan/${verb}`, { id }).catch(() => null)
          ));
          showToast(`${verb} sent for ${wpIds.length} workplan(s) under ${props.missionId}`, "ok");
        }
      } else if (verb === "mark-current-complete") {
        const f = currentFeature();
        const fid = f ? `${f.workplanId}:${f.id}` : "(none)";
        await restClient.post("/api/org/send-message", {
          from: "ceo",
          content: `@${chatPersona()} On mission ${props.missionId}: mark feature ${fid} as complete and advance. Operator override.`,
        });
        showToast(`Sent mark-complete for ${fid} → @${chatPersona()}`, "ok");
      } else if (verb === "re-assess") {
        await restClient.post("/api/org/send-message", {
          from: "ceo",
          content: `@${chatPersona()} On mission ${props.missionId}: re-assess remaining work, identify blockers, propose a revised plan. Operator override.`,
        });
        showToast(`Sent re-assess → @${chatPersona()}`, "ok");
      }
      await Promise.all([fetchWorkplans(), fetchEvents(), fetchChat()]);
    } catch (e: any) {
      showToast(`${verb} failed: ${e?.message ?? e}`, "err");
    } finally {
      setActionInFlight(null);
    }
  };

  const sendChat = async () => {
    const text = chatInput().trim();
    if (!text || chatSending()) return;
    setChatSending(true);
    try {
      await restClient.post("/api/org/send-message", {
        from: "ceo",
        content: `@${chatPersona()} ${text} (mission: ${props.missionId})`,
      });
      setChatInput("");
      await fetchChat();
      showToast(`Sent → @${chatPersona()}`, "ok");
    } catch (e: any) {
      showToast(`Send failed: ${e?.message ?? e}`, "err");
    } finally {
      setChatSending(false);
    }
  };

  const handleKey = (e: KeyboardEvent) => {
    const t = e.target as HTMLElement | null;
    if (t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable)) return;
    switch (e.key) {
      case "Escape": e.preventDefault(); navigate({ page: "missions" }); break;
      case "f": case "F": e.preventDefault(); flashPane(featuresPaneRef); break;
      case "l": case "L": e.preventDefault(); flashPane(logPaneRef); break;
      case "c": case "C": e.preventDefault(); flashPane(chatPaneRef); break;
      case "p": case "P": e.preventDefault(); doAction("pause"); break;
      case "r": case "R": e.preventDefault(); doAction("resume"); break;
      case "Tab": {
        e.preventDefault();
        const refs = [currentPaneRef, featuresPaneRef, logPaneRef, chatPaneRef].filter(Boolean) as HTMLDivElement[];
        if (refs.length === 0) break;
        const idx = (refs.findIndex((r) =>
          r.classList.contains("ring-2")) + (e.shiftKey ? -1 : 1) + refs.length) % refs.length;
        // Clear all rings before flashing the next
        refs.forEach((r) => r.classList.remove("ring-2", "ring-cyan-500/40"));
        flashPane(refs[idx]);
        break;
      }
    }
  };

  onMount(() => {
    Promise.all([fetchAdr(), fetchWorkplans(), fetchEvents(), fetchChat()]);
    timer = setInterval(() => {
      fetchAdr();
      fetchWorkplans();
      fetchEvents();
      fetchChat();
    }, REFRESH_MS);
    window.addEventListener("keydown", handleKey);
  });
  onCleanup(() => {
    if (timer) clearInterval(timer);
    window.removeEventListener("keydown", handleKey);
  });

  const tokenSummary = createMemo(() => {
    // Best-effort tally from /api/events. Each event has input/output_tokens
    // when it came from the inference router.
    let inp = 0, out = 0, n = 0;
    for (const e of events()) {
      const anyE = e as any;
      if (typeof anyE.input_tokens === "number") inp += anyE.input_tokens;
      if (typeof anyE.output_tokens === "number") out += anyE.output_tokens;
      if (typeof anyE.input_tokens === "number") n++;
    }
    const fmt = (n: number) => n >= 1_000_000 ? `${(n/1_000_000).toFixed(1)}M` : n >= 1000 ? `${(n/1000).toFixed(1)}K` : String(n);
    return { inp: fmt(inp), out: fmt(out), n };
  });

  return (
    <div class="h-screen flex flex-col bg-gray-950 text-gray-100 relative">
      {/* Toast */}
      <Show when={toast()}>
        <div class={`absolute top-3 right-3 z-50 px-3 py-2 rounded text-sm border shadow-lg ${
          toast()!.tone === "ok" ? "bg-green-900 text-green-100 border-green-700" :
          toast()!.tone === "err" ? "bg-red-900 text-red-100 border-red-700" :
          "bg-gray-800 text-gray-100 border-gray-700"
        }`}>
          {toast()!.text}
        </div>
      </Show>

      {/* Header */}
      <div class="shrink-0 border-b border-gray-800 px-4 py-2">
        <div class="flex items-center justify-between gap-4">
          <div class="flex items-center gap-3 min-w-0">
            <button
              onClick={() => navigate({ page: "missions" })}
              class="text-xs text-gray-500 hover:text-cyan-400 shrink-0"
              title="All missions (Esc)"
            >← All</button>
            <span class="text-sm font-semibold text-gray-200 shrink-0">Mission Control</span>
            <span class="text-xs text-gray-500 truncate font-mono">{props.missionId}</span>
          </div>
          <div class="flex items-center gap-4 text-xs text-gray-500 shrink-0">
            <span title="Workplans referencing this ADR">{workplans().length} workplans</span>
            <span title="Total features across all workplans">{counts().total} features</span>
            <Show when={tokenSummary().n > 0}>
              <span title={`${tokenSummary().n} inference calls observed`}>
                tokens: <span class="text-gray-300">{tokenSummary().inp}</span>
                <span class="text-gray-600"> in · </span>
                <span class="text-gray-300">{tokenSummary().out}</span>
                <span class="text-gray-600"> out</span>
              </span>
            </Show>
          </div>
        </div>
        <Show when={adr()}>
          <div class="mt-1.5 flex items-baseline gap-2">
            <h1 class="text-base font-semibold text-white truncate">{adr()!.title || adr()!.id}</h1>
            <span class={`text-[10px] px-1.5 py-0.5 rounded border shrink-0 ${ADR_STATUS_COLOR(adr()!.status)}`}>
              {adr()!.status || "—"}
            </span>
            <Show when={adr()!.date}>
              <span class="text-xs text-gray-600">{adr()!.date}</span>
            </Show>
          </div>
        </Show>
      </div>

      {/* Action bar + progress */}
      <div class="shrink-0 border-b border-gray-800 px-4 py-2 flex items-center gap-3">
        <Show when={isRunning()} fallback={
          <Show when={isComplete()} fallback={
            <span class="text-xs px-2 py-0.5 rounded border bg-gray-800 text-gray-400 border-gray-700">○ idle</span>
          }>
            <span class="text-xs px-2 py-0.5 rounded border bg-green-900 text-green-200 border-green-700">✓ Complete</span>
          </Show>
        }>
          <span class="text-xs px-2 py-0.5 rounded border bg-orange-900 text-orange-200 border-orange-700">● Running</span>
        </Show>
        <div class="flex-1 h-2 bg-gray-800 rounded-full overflow-hidden">
          <div
            class={`h-full transition-all ${isRunning() ? "bg-orange-500" : "bg-cyan-600"}`}
            style={{ width: `${pct()}%` }}
          />
        </div>
        <span class="text-xs font-mono text-gray-400 shrink-0">
          {counts().done}/{counts().total}
          <Show when={counts().failed > 0}>
            <span class="text-red-400 ml-2">{counts().failed} failed</span>
          </Show>
        </span>

        {/* Action buttons */}
        <div class="flex items-center gap-1 ml-2">
          <button
            onClick={() => doAction("pause")}
            disabled={!!actionInFlight() || workplans().length === 0}
            class="px-2 py-1 text-xs rounded border bg-gray-900 border-gray-700 text-gray-300 hover:bg-gray-800 hover:text-white disabled:opacity-40 disabled:cursor-not-allowed"
            title="P · Pause all workplans under this mission"
          >
            {actionInFlight() === "pause" ? "…" : "Pause"} <span class="text-gray-600 ml-1">P</span>
          </button>
          <button
            onClick={() => doAction("resume")}
            disabled={!!actionInFlight() || workplans().length === 0}
            class="px-2 py-1 text-xs rounded border bg-gray-900 border-gray-700 text-gray-300 hover:bg-gray-800 hover:text-white disabled:opacity-40 disabled:cursor-not-allowed"
            title="R · Resume all workplans under this mission"
          >
            {actionInFlight() === "resume" ? "…" : "Resume"} <span class="text-gray-600 ml-1">R</span>
          </button>
          <button
            onClick={() => doAction("mark-current-complete")}
            disabled={!!actionInFlight() || !currentFeature()}
            class="px-2 py-1 text-xs rounded border bg-gray-900 border-gray-700 text-gray-300 hover:bg-gray-800 hover:text-white disabled:opacity-40 disabled:cursor-not-allowed"
            title="Mark current feature complete (sent as directive to orchestrator persona — B7 backend pending)"
          >
            {actionInFlight() === "mark-current-complete" ? "…" : "Mark done"}
          </button>
          <button
            onClick={() => doAction("re-assess")}
            disabled={!!actionInFlight()}
            class="px-2 py-1 text-xs rounded border bg-gray-900 border-gray-700 text-gray-300 hover:bg-gray-800 hover:text-white disabled:opacity-40 disabled:cursor-not-allowed"
            title="Ask orchestrator to re-assess remaining work (B7 backend pending)"
          >
            {actionInFlight() === "re-assess" ? "…" : "Re-assess"}
          </button>
        </div>
      </div>

      <Show when={loading()}>
        <div class="p-6 text-gray-500">Loading mission…</div>
      </Show>
      <Show when={error()}>
        <div class="m-4 bg-red-950 border border-red-800 rounded-lg p-4 text-red-200">
          Failed to load: {error()}
        </div>
      </Show>

      {/* Body grid */}
      <div class="flex-1 min-h-0 grid grid-cols-12 gap-3 p-3">
        {/* Left: Current Feature */}
        <div ref={currentPaneRef} data-pane="current"
          class="col-span-4 bg-gray-900 border border-orange-900/40 rounded-lg overflow-hidden flex flex-col transition-shadow">
          <div class="px-4 py-2 border-b border-gray-800 text-xs uppercase tracking-wide text-orange-300/80">
            Current Feature
          </div>
          <Show when={currentFeature()} fallback={
            <div class="p-6 text-sm text-gray-500">
              <Show when={isComplete()} fallback={
                <Show when={workplans().length === 0} fallback="No feature in flight.">
                  No workplan references this ADR yet.
                </Show>
              }>Mission complete — all {counts().total} features done.</Show>
            </div>
          }>
            {(f) => (
              <div class="p-4 overflow-y-auto flex-1 text-sm">
                <div class="text-base font-semibold text-gray-100 mb-3 leading-snug">
                  {(f().name || "").trim()
                    || (f().files && f().files![0])
                    || `(task ${f().id})`}
                </div>
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
                    <span class="text-gray-500 mr-2">workplan</span>
                    <span class="text-gray-300 font-mono">{f().workplanId}</span>
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
                        <For each={f().files!}>{(file) => <li class="truncate">{file}</li>}</For>
                      </ul>
                    </div>
                  </Show>
                </div>
                <Show when={adr()?.body}>
                  <details class="mt-4 text-xs">
                    <summary class="text-gray-500 cursor-pointer hover:text-gray-300">ADR body</summary>
                    <pre class="mt-2 text-[11px] text-gray-400 whitespace-pre-wrap leading-relaxed">{adr()!.body!.slice(0, 1200)}{adr()!.body!.length > 1200 ? "…" : ""}</pre>
                  </details>
                </Show>
              </div>
            )}
          </Show>
        </div>

        {/* Middle: Features + Progress Log stacked */}
        <div class="col-span-5 flex flex-col gap-3 min-h-0">
          <div ref={featuresPaneRef} data-pane="features"
            class="bg-gray-900 border border-gray-800 rounded-lg overflow-hidden flex flex-col min-h-0 flex-1 transition-shadow">
            <div class="px-4 py-2 border-b border-gray-800 flex items-center justify-between text-xs">
              <span class="uppercase tracking-wide text-gray-500">Features</span>
              <span class="font-mono text-gray-400">
                {counts().done}/{counts().total}
                <Show when={counts().failed > 0}><span class="text-red-400 ml-2">{counts().failed} failed</span></Show>
                <Show when={counts().in_progress > 0}><span class="text-orange-400 ml-2">{counts().in_progress} in flight</span></Show>
              </span>
            </div>
            <div class="overflow-y-auto flex-1">
              <Show when={features().length > 0} fallback={
                <div class="p-3 text-xs text-gray-600 italic">No features yet — this ADR has no workplan.</div>
              }>
                <table class="w-full text-sm">
                  <thead class="sticky top-0 bg-gray-950 text-[10px] uppercase tracking-wide text-gray-600">
                    <tr>
                      <th class="px-2 py-1 text-left w-6"></th>
                      <th class="px-2 py-1 text-left w-16 font-medium">ID</th>
                      <th class="px-2 py-1 text-left font-medium">Feature</th>
                      <th class="px-2 py-1 text-left w-24 font-medium">Layer</th>
                      <th class="px-2 py-1 text-right w-10 font-medium">M</th>
                    </tr>
                  </thead>
                  <tbody class="divide-y divide-gray-850">
                    <For each={features()}>
                      {(f) => {
                        const key = `${f.workplanId}:${f.id}`;
                        const isCurrent = () => {
                          const c = currentFeature();
                          return c && `${c.workplanId}:${c.id}` === key;
                        };
                        // Tasks sometimes have no name field — fall back to first file
                        // or to the task id so the row is never blank.
                        const display = () => {
                          const n = (f.name || "").trim();
                          if (n) return n;
                          if (f.files && f.files.length > 0) return f.files[0];
                          return `(task ${f.id})`;
                        };
                        return (
                          <tr
                            onClick={() => setSelectedFeatureId(key)}
                            class={`cursor-pointer hover:bg-gray-850 ${isCurrent() ? "bg-orange-950/40" : ""}`}
                          >
                            <td class="px-2 py-1 align-top">
                              <StatusDot status={f.status} />
                            </td>
                            <td class="px-2 py-1 align-top font-mono text-[11px] text-gray-500 whitespace-nowrap">
                              {f.id}
                            </td>
                            <td class="px-2 py-1 align-top">
                              <div class={`truncate ${isCurrent() ? "text-orange-200" : "text-gray-200"}`}
                                   title={display()}>
                                {display()}
                              </div>
                              <Show when={!f.name && f.files && f.files.length > 1}>
                                <div class="text-[10px] text-gray-600 font-mono">+{f.files!.length - 1} more file{f.files!.length - 1 > 1 ? "s" : ""}</div>
                              </Show>
                            </td>
                            <td class="px-2 py-1 align-top text-[11px] text-gray-500 truncate" title={f.layer}>
                              {f.layer || "—"}
                            </td>
                            <td class="px-2 py-1 align-top text-right text-[10px] text-gray-600 font-mono whitespace-nowrap">
                              M{f.phaseIdx + 1}
                            </td>
                          </tr>
                        );
                      }}
                    </For>
                  </tbody>
                </table>
              </Show>
            </div>
          </div>

          <div ref={logPaneRef} data-pane="log"
            class="bg-gray-900 border border-gray-800 rounded-lg overflow-hidden flex flex-col min-h-0 flex-1 transition-shadow">
            <div class="px-4 py-2 border-b border-gray-800 flex items-center justify-between text-xs">
              <span class="uppercase tracking-wide text-gray-500">Progress Log</span>
              <div class="flex items-center gap-2">
                <button
                  onClick={() => { setShowHeartbeats(!showHeartbeats()); fetchEvents(); }}
                  class={`text-[10px] px-2 py-0.5 rounded border ${
                    showHeartbeats()
                      ? "bg-gray-800 text-gray-300 border-gray-700"
                      : "bg-gray-950 text-gray-600 border-gray-800 hover:text-gray-400"
                  }`}
                  title="Show / hide heartbeat ticks (brain_tick, adr_doctor_tick, etc.)"
                >
                  {showHeartbeats() ? "noise on" : "noise off"}
                </button>
                <span class="font-mono text-gray-600">{events().length} events</span>
              </div>
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
                        {e.event_type === "PostToolUse" ? "✓" : e.event_type === "PreToolUse" ? "●" : "·"}
                      </span>
                      <span class="text-gray-400 shrink-0">{e.event_type}</span>
                      <Show when={e.tool_name}>
                        <span class="text-gray-300 truncate">{e.tool_name}</span>
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

        {/* Right: Orchestrator chat */}
        <div ref={chatPaneRef} data-pane="chat"
          class="col-span-3 bg-gray-900 border border-gray-800 rounded-lg overflow-hidden flex flex-col min-h-0 transition-shadow">
          <div class="px-3 py-2 border-b border-gray-800 flex items-center justify-between gap-2 text-xs">
            <span class="uppercase tracking-wide text-gray-500">Orchestrator</span>
            <select
              value={chatPersona()}
              onChange={(e) => setChatPersona(e.currentTarget.value as any)}
              class="bg-gray-950 border border-gray-800 rounded px-1 py-0.5 text-xs text-gray-300"
            >
              <option value="engineering-lead">@engineering-lead</option>
              <option value="cto">@cto</option>
              <option value="cpo">@cpo</option>
              <option value="coo">@coo</option>
            </select>
          </div>
          <div class="overflow-y-auto flex-1 p-2 space-y-2 text-xs">
            <Show when={chatMessages().length > 0} fallback={
              <div class="text-gray-600 italic">No conversation yet for this mission.</div>
            }>
              <For each={chatMessages()}>
                {(m) => (
                  <div class={`p-2 rounded border ${
                    m.from === "ceo"
                      ? "bg-cyan-950/40 border-cyan-900/40 ml-4"
                      : "bg-gray-950 border-gray-800 mr-4"
                  }`}>
                    <div class="text-[10px] text-gray-500 mb-0.5">
                      {m.from === "ceo" ? "You" : m.from} → {m.to}
                    </div>
                    <div class="text-gray-200 whitespace-pre-wrap break-words">
                      {m.content.replace(/^\[[\w-]+\][^\n]*?(?:→[^\n]*?\(rounds=\d+\)\s*)?/, "")}
                    </div>
                  </div>
                )}
              </For>
            </Show>
          </div>
          <div class="border-t border-gray-800 p-2">
            <textarea
              value={chatInput()}
              onInput={(e) => setChatInput(e.currentTarget.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  sendChat();
                }
              }}
              rows={2}
              placeholder={`Message @${chatPersona()}… (⌘↵)`}
              class="w-full bg-gray-950 border border-gray-800 rounded px-2 py-1 text-xs text-gray-200 placeholder-gray-600 resize-none focus:border-cyan-700 focus:outline-none"
            />
            <button
              onClick={sendChat}
              disabled={chatSending() || !chatInput().trim()}
              class="mt-1 w-full px-2 py-1 text-xs rounded bg-cyan-700 hover:bg-cyan-600 disabled:opacity-40 disabled:cursor-not-allowed text-white"
            >
              {chatSending() ? "Sending…" : "Send"}
            </button>
          </div>
        </div>
      </div>

      {/* Bottom: raw stream */}
      <div class="shrink-0 border-t border-gray-800 bg-gray-950 max-h-40 overflow-y-auto">
        <div class="px-4 py-1.5 border-b border-gray-800 text-xs uppercase tracking-wide text-gray-500 sticky top-0 bg-gray-950">
          Raw stream
        </div>
        <div class="font-mono text-[11px] leading-relaxed p-2">
          <Show when={events().length > 0} fallback={<div class="text-gray-700 italic">No raw events yet.</div>}>
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

      {/* Footer keyboard hint bar (truth-only) */}
      <div class="shrink-0 border-t border-gray-800 px-4 py-1 text-[11px] text-gray-600 flex gap-4">
        <span><span class="text-gray-400">Esc</span> back</span>
        <span><span class="text-gray-400">F</span> features</span>
        <span><span class="text-gray-400">L</span> log</span>
        <span><span class="text-gray-400">C</span> chat</span>
        <span><span class="text-gray-400">P</span> pause</span>
        <span><span class="text-gray-400">R</span> resume</span>
        <span><span class="text-gray-400">Tab</span> cycle</span>
        <span><span class="text-gray-400">⌘↵</span> send</span>
      </div>
    </div>
  );
};

// ── Router shell ────────────────────────────────────────────────────────────

const Missions: Component = () => {
  return (
    <Show when={route().page === "mission-detail"} fallback={<MissionsList />}>
      <MissionDetailView missionId={(route() as any).missionId} />
    </Show>
  );
};

export default Missions;
