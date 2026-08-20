/**
 * Thoughts.tsx — agent thought stream.
 *
 * Tails chat-relay.agent_thought across all personas, with role and kind
 * filters. Lets the operator audit the inner monologue that the org_responder
 * is journaling on each user prompt (see project_phase2_thought_memory.md).
 */

import { Component, For, Show, createSignal, onMount, onCleanup, createMemo } from "solid-js";
import { restClient } from "../../services/rest-client";

interface Thought {
  thought_id: number;
  agent_role: string;
  kind: string;
  content: string;
  related_task_id: string;
  related_msg_id: number;
  confidence: number;
  created_at: string;
}

interface ThoughtList {
  thoughts: Thought[];
}

const REFRESH_MS = 4000;

const KINDS = ["", "decision", "observation", "plan", "frustration", "learning", "commitment"];

// STDB Timestamp { __timestamp_micros_since_unix_epoch__: N } → ISO display.
const fmtTimestamp = (ts: string): string => {
  if (!ts) return "";
  const m = ts.match(/__timestamp_micros_since_unix_epoch__:\s*(-?\d+)/);
  if (!m) return ts;
  const n = Number(m[1]);
  if (!Number.isFinite(n) || n === 0) return "";
  const d = new Date(n / 1000);
  return d.toLocaleString();
};

const kindColor = (k: string) => {
  switch (k) {
    case "decision":
      return "text-cyan-400";
    case "observation":
      return "text-blue-400";
    case "plan":
      return "text-purple-400";
    case "frustration":
      return "text-orange-400";
    case "learning":
      return "text-green-400";
    case "commitment":
      return "text-yellow-400";
    default:
      return "text-gray-400";
  }
};

const Thoughts: Component = () => {
  const [thoughts, setThoughts] = createSignal<Thought[]>([]);
  const [roles, setRoles] = createSignal<string[]>([]);
  const [roleFilter, setRoleFilter] = createSignal<string>("");
  const [kindFilter, setKindFilter] = createSignal<string>("");
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);

  let timer: ReturnType<typeof setInterval> | null = null;

  const refresh = async () => {
    try {
      const params = new URLSearchParams();
      if (roleFilter()) params.set("role", roleFilter());
      if (kindFilter()) params.set("kind", kindFilter());
      params.set("limit", "200");
      const data: ThoughtList = await restClient.get(
        `/api/merge/thoughts?${params.toString()}`,
      );
      const ts = data.thoughts || [];
      setThoughts(ts);
      const seen = new Set<string>();
      ts.forEach((t) => seen.add(t.agent_role));
      setRoles(Array.from(seen).sort());
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

  // Refresh whenever a filter changes
  createMemo(() => {
    roleFilter();
    kindFilter();
    setLoading(true);
    refresh();
  });

  return (
    <div class="flex flex-col bg-gray-950 min-h-screen text-gray-100">
      <div class="p-6 border-b border-gray-800">
        <h1 class="text-2xl font-bold mb-1">Thought Stream</h1>
        <p class="text-gray-400 text-sm">
          chat-relay · agent_thought · auto-journaled on every prompt
        </p>
      </div>

      <div class="px-6 py-3 border-b border-gray-900 flex gap-3 items-center">
        <label class="text-xs text-gray-500 uppercase">role</label>
        <select
          class="bg-gray-900 border border-gray-800 rounded px-2 py-1 text-sm"
          value={roleFilter()}
          onChange={(e) => setRoleFilter(e.currentTarget.value)}
        >
          <option value="">all</option>
          <For each={roles()}>{(r) => <option value={r}>{r}</option>}</For>
        </select>

        <label class="text-xs text-gray-500 uppercase ml-4">kind</label>
        <select
          class="bg-gray-900 border border-gray-800 rounded px-2 py-1 text-sm"
          value={kindFilter()}
          onChange={(e) => setKindFilter(e.currentTarget.value)}
        >
          <For each={KINDS}>
            {(k) => <option value={k}>{k || "all"}</option>}
          </For>
        </select>

        <span class="ml-auto text-xs text-gray-500">{thoughts().length} entries</span>
      </div>

      <Show when={error()}>
        <div class="p-4 bg-red-950/40 border-b border-red-900 text-red-300 text-sm">
          {error()}
        </div>
      </Show>

      <Show when={loading() && thoughts().length === 0}>
        <div class="p-6 text-gray-500">Loading thoughts…</div>
      </Show>

      <Show when={!loading() && thoughts().length === 0}>
        <div class="p-6 text-gray-500">No thoughts match the current filters.</div>
      </Show>

      <div class="flex-1 overflow-y-auto px-6 py-4 space-y-2 font-mono text-sm">
        <For each={thoughts()}>
          {(t) => (
            <div class="border border-gray-900 rounded bg-gray-900/40 p-3">
              <div class="flex items-baseline gap-3 text-xs">
                <span class="text-gray-500">#{t.thought_id}</span>
                <span class="text-cyan-400">{t.agent_role}</span>
                <span class={kindColor(t.kind)}>{t.kind}</span>
                <Show when={t.confidence > 0}>
                  <span class="text-gray-500">
                    conf {(t.confidence * 100).toFixed(0)}%
                  </span>
                </Show>
                <Show when={t.related_msg_id > 0}>
                  <span class="text-gray-500">msg {t.related_msg_id}</span>
                </Show>
                <span class="text-gray-600 ml-auto">{fmtTimestamp(t.created_at)}</span>
              </div>
              <div class="mt-1 text-gray-200 whitespace-pre-wrap">{t.content}</div>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

export default Thoughts;
