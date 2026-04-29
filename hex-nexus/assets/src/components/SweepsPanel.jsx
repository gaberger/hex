/**
 * SweepsPanel.jsx — Idle-research sweep summary card
 * (workplan `wp-idle-research-swarm`, P5.2).
 *
 * Reads `GET /api/research/sweeps` and renders one row per sweep showing:
 *   - timestamp (relative + absolute)
 *   - finding count
 *   - draft count
 *   - per-finding draft links (ADR drafts + workplan drafts)
 *
 * Lightweight by design: a single fetch, a 30s polling interval, no
 * WebSocket subscription. Sweeps are throttled to ≥6h apart so the dashboard
 * doesn't need sub-second freshness — stale-by-30s is fine.
 *
 * The card mounts as a child of the main dashboard (ControlPlane) and
 * collapses to "No sweeps yet" on a fresh project where the idle-research
 * coordinator has not yet produced any artifacts.
 */
import { For, Show, createResource, createSignal, onCleanup, onMount } from "solid-js";

const POLL_INTERVAL_MS = 30_000;
const DEFAULT_LIMIT = 5;

async function fetchSweeps(limit) {
  const res = await fetch(`/api/research/sweeps?limit=${limit}`);
  if (!res.ok) {
    throw new Error(`HTTP ${res.status}`);
  }
  const body = await res.json();
  return Array.isArray(body?.sweeps) ? body.sweeps : [];
}

function relativeTime(rfc3339OrStem) {
  if (!rfc3339OrStem) return "—";
  const d = new Date(rfc3339OrStem);
  if (Number.isNaN(d.getTime())) {
    // Handler falls back to the filename stem (idle-sweep-YYYYMMDD-HHMM)
    // when the YAML header is missing — show it verbatim.
    return rfc3339OrStem;
  }
  const diff = Date.now() - d.getTime();
  if (diff < 0) return "just now";
  if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)}h ago`;
  return `${Math.floor(diff / 86_400_000)}d ago`;
}

function absoluteTime(rfc3339OrStem) {
  if (!rfc3339OrStem) return "";
  const d = new Date(rfc3339OrStem);
  if (Number.isNaN(d.getTime())) return rfc3339OrStem;
  const pad = (n) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ` +
    `${pad(d.getHours())}:${pad(d.getMinutes())}`
  );
}

function kindBadgeClass(kind) {
  switch (kind) {
    case "draft-adr":
      return "bg-purple-900/40 text-purple-300 border-purple-800";
    case "draft-workplan":
      return "bg-blue-900/40 text-blue-300 border-blue-800";
    case "amend-workplan":
      return "bg-cyan-900/40 text-cyan-300 border-cyan-800";
    case "memory":
      return "bg-amber-900/40 text-amber-300 border-amber-800";
    default:
      return "bg-gray-900/40 text-gray-400 border-gray-800";
  }
}

function DraftLink(props) {
  // `props.draft` shape: { finding_id, kind, path? }
  return (
    <Show
      when={props.draft.path}
      fallback={
        <span
          class={`inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-[10px] ${kindBadgeClass(
            props.draft.kind,
          )}`}
          title={`${props.draft.finding_id} — no draft file (informational/memory)`}
        >
          {props.draft.kind}
        </span>
      }
    >
      <a
        href={`/${props.draft.path}`}
        class={`inline-flex items-center gap-1 rounded border px-1.5 py-0.5 text-[10px] hover:underline ${kindBadgeClass(
          props.draft.kind,
        )}`}
        title={`${props.draft.finding_id} → ${props.draft.path}`}
        target="_blank"
        rel="noopener"
      >
        {props.draft.kind}
      </a>
    </Show>
  );
}

function SweepRow(props) {
  // `props.sweep` shape: { yaml_path, markdown_path, sweep_at,
  //                       finding_count, draft_count, drafts[] }
  return (
    <li class="flex flex-col gap-1.5 border-b border-gray-800 px-3 py-2 last:border-b-0">
      <div class="flex items-center justify-between gap-2">
        <div class="flex items-center gap-2 text-xs">
          <span class="font-medium text-gray-200">
            {relativeTime(props.sweep.sweep_at)}
          </span>
          <span class="text-gray-500">{absoluteTime(props.sweep.sweep_at)}</span>
        </div>
        <div class="flex items-center gap-2 text-[11px]">
          <span class="rounded bg-gray-800 px-1.5 py-0.5 text-gray-300">
            {props.sweep.finding_count} finding
            {props.sweep.finding_count === 1 ? "" : "s"}
          </span>
          <Show when={props.sweep.draft_count > 0}>
            <span class="rounded bg-blue-900/40 px-1.5 py-0.5 text-blue-300 border border-blue-800">
              {props.sweep.draft_count} draft
              {props.sweep.draft_count === 1 ? "" : "s"}
            </span>
          </Show>
        </div>
      </div>
      <Show when={props.sweep.drafts.length > 0}>
        <div class="flex flex-wrap items-center gap-1">
          <For each={props.sweep.drafts}>
            {(d) => <DraftLink draft={d} />}
          </For>
        </div>
      </Show>
      <div class="flex items-center gap-3 text-[10px] text-gray-500">
        <a
          href={`/${props.sweep.markdown_path}`}
          class="hover:text-gray-300 hover:underline"
          target="_blank"
          rel="noopener"
        >
          summary.md
        </a>
        <a
          href={`/${props.sweep.yaml_path}`}
          class="hover:text-gray-300 hover:underline"
          target="_blank"
          rel="noopener"
        >
          sweep.yaml
        </a>
      </div>
    </li>
  );
}

const SweepsPanel = (props) => {
  const limit = () => props?.limit ?? DEFAULT_LIMIT;
  const [tick, setTick] = createSignal(0);
  const [sweeps] = createResource(
    () => ({ limit: limit(), tick: tick() }),
    ({ limit }) => fetchSweeps(limit),
  );

  let timer;
  onMount(() => {
    timer = setInterval(() => setTick((t) => t + 1), POLL_INTERVAL_MS);
  });
  onCleanup(() => {
    if (timer) clearInterval(timer);
  });

  return (
    <section class="rounded-lg border border-gray-800 bg-gray-950/40">
      <header class="flex items-center justify-between border-b border-gray-800 px-3 py-2">
        <div class="flex items-center gap-2">
          <span class="h-1.5 w-1.5 rounded-full bg-purple-400" />
          <h3 class="text-sm font-medium text-gray-200">Idle research sweeps</h3>
        </div>
        <Show when={!sweeps.loading && sweeps()}>
          <span class="text-[10px] uppercase tracking-wide text-gray-500">
            last {Math.min(sweeps()?.length ?? 0, limit())} of {limit()}
          </span>
        </Show>
      </header>
      <Show
        when={!sweeps.loading}
        fallback={
          <div class="px-3 py-4 text-xs text-gray-500">Loading sweeps…</div>
        }
      >
        <Show
          when={sweeps.error}
          fallback={
            <Show
              when={(sweeps() ?? []).length > 0}
              fallback={
                <div class="px-3 py-4 text-xs text-gray-500">
                  No sweeps yet — the idle-research coordinator runs after the
                  sched queue has been idle for ≥4 ticks (and ≥6h since the
                  last sweep).
                </div>
              }
            >
              <ul class="divide-y divide-gray-800">
                <For each={sweeps()}>{(s) => <SweepRow sweep={s} />}</For>
              </ul>
            </Show>
          }
        >
          <div class="px-3 py-4 text-xs text-red-400">
            Failed to load sweeps: {String(sweeps.error?.message ?? sweeps.error)}
          </div>
        </Show>
      </Show>
    </section>
  );
};

export default SweepsPanel;
