/**
 * BrainDecisions.tsx — wp-brain-dashboard M1.
 *
 * Surfaces every human-in-loop bottleneck the operator owes a decision on:
 *   - blocked workplan tasks
 *   - ADRs aging in Status: Proposed
 *   - persona-bypass dispatch arms (Gap B from `hex agent overview`)
 *   - unacked priority-2 inbox notifications (ADR-060)
 *
 * Data: GET /api/decisions (aggregated by hex-nexus from filesystem + STDB).
 * Refreshes on mount and every 30s.
 */
import { Component, For, Show, createSignal, onCleanup, onMount, createMemo } from "solid-js";
import { restClient } from "../../services/rest-client";
import { addToast } from "../../stores/toast";

// ---------------------------------------------------------------------------
// Types — mirror hex-nexus/src/routes/decisions.rs DecisionItem
// ---------------------------------------------------------------------------

interface DecisionItem {
  id: string;
  kind: string; // blocked_task | proposed_adr | persona_bypass | priority_inbox
  severity: "CRITICAL" | "HIGH" | "MEDIUM" | "LOW";
  title: string;
  reason: string;
  ageSeconds: number;
  suggestedAction: string;
  link: string | null;
}

interface DecisionsResponse {
  items: DecisionItem[];
  total: number;
  bySeverity: Record<string, number>;
  generatedAt: string;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function severityBadge(severity: string): string {
  switch (severity) {
    case "CRITICAL": return "bg-red-900/40 text-red-300 border-red-700";
    case "HIGH":     return "bg-orange-900/40 text-orange-300 border-orange-700";
    case "MEDIUM":   return "bg-yellow-900/30 text-yellow-300 border-yellow-700";
    case "LOW":      return "bg-gray-800 text-gray-400 border-gray-700";
    default:         return "bg-gray-800 text-gray-400 border-gray-700";
  }
}

function kindIcon(kind: string): string {
  switch (kind) {
    case "blocked_task":     return "⛔";
    case "proposed_adr":     return "📜";
    case "persona_bypass":   return "🛑";
    case "priority_inbox":   return "📬";
    case "adversary_disagreement": return "⚖";
    case "reconcile_demotion":     return "↩";
    default: return "•";
  }
}

function formatAge(seconds: number): string {
  if (!seconds || seconds <= 0) return "—";
  if (seconds < 60) return `${seconds}s`;
  const mins = Math.floor(seconds / 60);
  if (mins < 60) return `${mins}m`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h`;
  return `${Math.floor(hrs / 24)}d`;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

const BrainDecisions: Component = () => {
  const [data, setData] = createSignal<DecisionsResponse | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [filter, setFilter] = createSignal<string | null>(null);

  let pollHandle: number | undefined;

  const fetchDecisions = async () => {
    try {
      const resp = await restClient.get<DecisionsResponse>("/api/decisions");
      setData(resp);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    fetchDecisions();
    // Refresh every 30s — decisions don't change at sub-second pacing.
    pollHandle = window.setInterval(fetchDecisions, 30000);
  });
  onCleanup(() => {
    if (pollHandle !== undefined) window.clearInterval(pollHandle);
  });

  const filteredItems = createMemo(() => {
    const d = data();
    if (!d) return [];
    const f = filter();
    if (!f) return d.items;
    return d.items.filter((it) => it.severity === f);
  });

  const handleResolve = async (item: DecisionItem) => {
    if (item.kind === "priority_inbox") {
      // inbox:<id> → POST /api/decisions/<id>
      const id = item.id.split(":")[1];
      try {
        await restClient.post(`/api/decisions/${id}`, { action: "approve" });
        addToast({ kind: "success", message: `Acknowledged ${item.title}` });
        fetchDecisions();
      } catch (e) {
        addToast({ kind: "error", message: `Failed to acknowledge: ${e}` });
      }
    } else {
      // For other kinds, navigate to the artifact.
      if (item.link) {
        window.location.hash = item.link;
      } else {
        addToast({
          kind: "info",
          message: `${item.kind} decisions resolve in the artifact (${item.id})`,
        });
      }
    }
  };

  return (
    <div class="p-6 max-w-5xl mx-auto">
      <header class="mb-6">
        <h1 class="text-2xl font-bold text-gray-100 mb-1">Decisions</h1>
        <p class="text-sm text-gray-400">
          Human-in-loop bottlenecks across the agent surface. Refreshes every 30s.
        </p>
      </header>

      <Show when={loading()}>
        <div class="text-gray-400">Loading...</div>
      </Show>

      <Show when={error()}>
        <div class="bg-red-900/30 border border-red-700 text-red-300 p-4 rounded mb-4">
          <strong>Failed to load decisions:</strong> {error()}
        </div>
      </Show>

      <Show when={data()}>
        {(d) => (
          <>
            {/* Summary bar */}
            <div class="flex gap-2 mb-6 flex-wrap">
              <button
                onClick={() => setFilter(null)}
                class={`px-3 py-1.5 rounded text-sm font-medium border transition ${
                  filter() === null
                    ? "bg-gray-700 text-gray-100 border-gray-600"
                    : "bg-gray-900 text-gray-400 border-gray-800 hover:bg-gray-800"
                }`}
              >
                All ({d().total})
              </button>
              <For each={["CRITICAL", "HIGH", "MEDIUM", "LOW"]}>
                {(sev) => (
                  <Show when={(d().bySeverity[sev] || 0) > 0}>
                    <button
                      onClick={() => setFilter(filter() === sev ? null : sev)}
                      class={`px-3 py-1.5 rounded text-sm font-medium border transition ${severityBadge(sev)} ${
                        filter() === sev ? "ring-2 ring-offset-1 ring-offset-gray-950 ring-white/30" : ""
                      }`}
                    >
                      {sev} ({d().bySeverity[sev] || 0})
                    </button>
                  </Show>
                )}
              </For>
            </div>

            {/* Items */}
            <Show
              when={filteredItems().length > 0}
              fallback={
                <div class="text-center py-12 text-gray-300">
                  <div class="text-4xl mb-2">✓</div>
                  <p class="text-lg">No decisions pending — you're caught up.</p>
                </div>
              }
            >
              <ul class="space-y-3">
                <For each={filteredItems()}>
                  {(item) => (
                    <li class="bg-gray-900 border border-gray-800 rounded-lg p-4 hover:border-gray-700 transition">
                      <div class="flex items-start justify-between gap-4">
                        <div class="flex-1 min-w-0">
                          <div class="flex items-center gap-2 mb-1">
                            <span class="text-xl flex-shrink-0">{kindIcon(item.kind)}</span>
                            <span
                              class={`text-xs font-bold px-2 py-0.5 rounded border ${severityBadge(item.severity)}`}
                            >
                              {item.severity}
                            </span>
                            <span class="text-xs text-gray-300 uppercase tracking-wide">
                              {item.kind.replace(/_/g, " ")}
                            </span>
                            <span class="text-xs text-gray-400 ml-auto flex-shrink-0">
                              {formatAge(item.ageSeconds)}
                            </span>
                          </div>
                          <h3 class="text-sm font-semibold text-gray-100 mb-1 truncate">
                            {item.title}
                          </h3>
                          <p class="text-xs text-gray-400 leading-relaxed">{item.reason}</p>
                        </div>
                        <button
                          onClick={() => handleResolve(item)}
                          class="px-3 py-1.5 text-xs font-medium bg-blue-900/30 text-blue-300 border border-blue-700 rounded hover:bg-blue-900/50 transition flex-shrink-0"
                        >
                          {item.suggestedAction}
                        </button>
                      </div>
                    </li>
                  )}
                </For>
              </ul>
            </Show>

            <div class="text-xs text-gray-400 mt-6 text-right">
              Last fetched: {new Date(d().generatedAt).toLocaleTimeString()}
            </div>
          </>
        )}
      </Show>
    </div>
  );
};

export default BrainDecisions;
