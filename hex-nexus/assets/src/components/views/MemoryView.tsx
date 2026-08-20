/**
 * MemoryView.tsx — the agent learning loop (ADR-2606061359).
 *
 * Surfaces the `hexflo_memory` table (already WS-subscribed via stores/connection)
 * — `lesson:` (don't-repeat), `gap:` (system blind spots), `project:` context,
 * `action:`/`failure:` history. This is the Hermes/OpenClaw-style memory that the
 * single-agent loop reasons with; previously it had no dashboard surface.
 */
import { Component, For, Show, createSignal, createMemo } from "solid-js";
import { hexfloMemory } from "../../stores/connection";

interface MemRow { key: string; value: string; scope: string; updated_at: string }

const prefixOf = (key: string) => {
  const i = key.indexOf(":");
  return i > 0 ? key.slice(0, i) : "other";
};

const PREFIX_TONE: Record<string, string> = {
  lesson: "text-green-400",
  gap: "text-amber-400",
  project: "text-cyan-400",
  action: "text-gray-400",
  failure: "text-red-400",
  nopact: "text-gray-500",
};

const MemoryView: Component = () => {
  const [q, setQ] = createSignal("");

  const grouped = createMemo(() => {
    const rows = (hexfloMemory() as MemRow[]) ?? [];
    const needle = q().trim().toLowerCase();
    const filtered = needle
      ? rows.filter((r) => r.key.toLowerCase().includes(needle) || (r.value ?? "").toLowerCase().includes(needle))
      : rows;
    const groups: Record<string, MemRow[]> = {};
    for (const r of filtered) {
      (groups[prefixOf(r.key)] ||= []).push(r);
    }
    // Stable, meaningful order; lessons + gaps first.
    const order = ["lesson", "gap", "project", "failure", "action", "nopact", "other"];
    return Object.entries(groups).sort(
      (a, b) => (order.indexOf(a[0]) + 1 || 99) - (order.indexOf(b[0]) + 1 || 99),
    );
  });

  const total = createMemo(() => ((hexfloMemory() as MemRow[]) ?? []).length);

  return (
    <div class="flex flex-1 flex-col overflow-auto p-4 gap-4 text-sm text-gray-200">
      <div class="flex items-center gap-3 flex-wrap">
        <h1 class="text-lg font-semibold text-gray-100">Memory</h1>
        <span class="text-gray-500 text-xs">{total()} entries — what the agents have learned</span>
        <input
          class="ml-auto rounded border border-gray-700 bg-gray-900 px-2 py-1 text-xs w-64"
          placeholder="search keys + values"
          value={q()}
          onInput={(e) => setQ(e.currentTarget.value)}
        />
      </div>

      <Show
        when={grouped().length > 0}
        fallback={<span class="text-gray-600 text-xs">No memory entries{q() ? " match." : " yet — agents write lessons/gaps as they run."}</span>}
      >
        <For each={grouped()}>
          {([prefix, rows]) => (
            <div class="rounded border border-gray-800 bg-gray-900/50 p-3">
              <div class="text-xs uppercase tracking-wide mb-2" classList={{ [PREFIX_TONE[prefix] ?? "text-gray-400"]: true }}>
                {prefix} <span class="text-gray-600">({rows.length})</span>
              </div>
              <div class="flex flex-col gap-1.5">
                <For each={rows}>
                  {(r) => (
                    <div class="flex flex-col gap-0.5 border-b border-gray-900 pb-1.5 last:border-0">
                      <div class="flex items-center gap-2">
                        <span class="font-mono text-[11px] text-gray-300">{r.key}</span>
                        <Show when={r.scope && r.scope !== "global"}>
                          <span class="text-[10px] text-gray-600">[{r.scope}]</span>
                        </Show>
                      </div>
                      <div class="text-xs text-gray-400 whitespace-pre-wrap">{r.value}</div>
                    </div>
                  )}
                </For>
              </div>
            </div>
          )}
        </For>
      </Show>
    </div>
  );
};

export default MemoryView;
