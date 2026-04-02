/**
 * ActivityPanel.tsx — Live tool-call event timeline (ADR-2604012137).
 *
 * Shows PreToolUse+PostToolUse pairs as a collapsible timeline.
 * Loads recent history from GET /api/events, then streams new events
 * via the main /ws WebSocket (topic: "events", event: "tool_event").
 */
import {
  Component,
  For,
  Show,
  createSignal,
  onMount,
  onCleanup,
  createMemo,
} from "solid-js";
import { restClient } from "../../services/rest-client";

// ── Types ────────────────────────────────────────────────────────────────────

interface ToolEvent {
  id: number;
  session_id: string;
  agent_id: string | null;
  event_type: string;
  tool_name: string | null;
  input_json: string | null;
  result_json: string | null;
  exit_code: number | null;
  duration_ms: number | null;
  model_used: string | null;
  context_strategy: string | null;
  rl_action: string | null;
  input_tokens: number | null;
  output_tokens: number | null;
  cost_usd: number | null;
  hex_layer: string | null;
  created_at: string;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

function relativeTime(ts: string): string {
  const diff = Date.now() - new Date(ts).getTime();
  if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  return `${Math.floor(diff / 3_600_000)}h ago`;
}

function formatDatetime(ts: string): string {
  const d = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, "0");
  return (
    `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ` +
    `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
  );
}

function eventTypeColor(t: string): string {
  switch (t) {
    case "PreToolUse": return "text-blue-400";
    case "PostToolUse": return "text-green-400";
    case "SubagentStart": return "text-purple-400";
    case "SubagentStop": return "text-purple-300";
    case "Stop": return "text-gray-400";
    default: return "text-gray-300";
  }
}

function formatCost(usd: number | null): string {
  if (usd == null) return "—";
  if (usd < 0.001) return `$${(usd * 1000).toFixed(3)}m`;
  return `$${usd.toFixed(4)}`;
}

// ── Row component ─────────────────────────────────────────────────────────────

const EventRow: Component<{ event: ToolEvent }> = (props) => {
  const [expanded, setExpanded] = createSignal(false);
  const e = props.event;

  return (
    <div class="border-b border-gray-800 hover:bg-gray-900/40">
      {/* Summary row */}
      <div
        class="flex items-center gap-3 px-4 py-2 cursor-pointer text-sm"
        onClick={() => setExpanded(!expanded())}
      >
        {/* Expand toggle */}
        <span class="text-gray-600 w-3 select-none">
          {expanded() ? "▾" : "▸"}
        </span>
        {/* Event type */}
        <span class={`w-28 shrink-0 font-mono text-xs ${eventTypeColor(e.event_type)}`}>
          {e.event_type}
        </span>
        {/* Tool name */}
        <span class="text-gray-200 font-mono text-xs w-32 shrink-0 truncate">
          {e.tool_name ?? "—"}
        </span>
        {/* Model */}
        <span class="text-gray-500 text-xs w-40 shrink-0 truncate">
          {e.model_used ?? "—"}
        </span>
        {/* Latency */}
        <span class="text-gray-500 text-xs w-16 shrink-0 text-right">
          {e.duration_ms != null ? `${e.duration_ms}ms` : "—"}
        </span>
        {/* Cost */}
        <span class="text-yellow-600 text-xs w-20 shrink-0 text-right">
          {formatCost(e.cost_usd)}
        </span>
        {/* Tokens */}
        <span class="text-gray-500 text-xs w-24 shrink-0 text-right">
          {e.input_tokens != null ? `↑${e.input_tokens}` : ""}
          {e.output_tokens != null ? ` ↓${e.output_tokens}` : ""}
        </span>
        {/* Timestamp */}
        <span class="text-right ml-auto shrink-0 leading-tight">
          <span class="block text-gray-400 text-xs font-mono">{formatDatetime(e.created_at)}</span>
          <span class="block text-gray-600 text-xs">{relativeTime(e.created_at)}</span>
        </span>
      </div>

      {/* Expanded detail */}
      <Show when={expanded()}>
        <div class="px-8 pb-3 grid grid-cols-2 gap-3 text-xs">
          <Show when={e.input_json}>
            <div>
              <div class="text-gray-500 mb-1">Input</div>
              <pre class="bg-gray-950 rounded p-2 overflow-auto max-h-40 text-gray-300 font-mono text-xs">
                {e.input_json}
              </pre>
            </div>
          </Show>
          <Show when={e.result_json}>
            <div>
              <div class="text-gray-500 mb-1">Result</div>
              <pre class="bg-gray-950 rounded p-2 overflow-auto max-h-40 text-gray-300 font-mono text-xs">
                {e.result_json}
              </pre>
            </div>
          </Show>
          <Show when={e.rl_action || e.context_strategy || e.hex_layer}>
            <div class="col-span-2 flex gap-4 text-gray-500">
              <Show when={e.hex_layer}>
                <span>Layer: <span class="text-gray-300">{e.hex_layer}</span></span>
              </Show>
              <Show when={e.context_strategy}>
                <span>Context: <span class="text-gray-300">{e.context_strategy}</span></span>
              </Show>
              <Show when={e.rl_action}>
                <span>RL: <span class="text-gray-300">{e.rl_action}</span></span>
              </Show>
              <Show when={e.agent_id}>
                <span>Agent: <span class="text-gray-300">{e.agent_id}</span></span>
              </Show>
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
};

// ── Main panel ────────────────────────────────────────────────────────────────

const ActivityPanel: Component = () => {
  const [events, setEvents] = createSignal<ToolEvent[]>([]);
  const [sessionFilter, setSessionFilter] = createSignal("");
  const [agentFilter, setAgentFilter] = createSignal("");
  const [loading, setLoading] = createSignal(true);
  const [wsConnected, setWsConnected] = createSignal(false);

  // Apply filters
  const filtered = createMemo(() => {
    const sf = sessionFilter().toLowerCase();
    const af = agentFilter().toLowerCase();
    return events().filter((e) => {
      if (sf && !e.session_id.toLowerCase().includes(sf)) return false;
      if (af && !(e.agent_id ?? "").toLowerCase().includes(af)) return false;
      return true;
    });
  });

  // Load initial history
  async function loadHistory() {
    setLoading(true);
    try {
      const resp: any = await restClient.get("/api/events?limit=100");
      if (resp.events) {
        setEvents(resp.events as ToolEvent[]);
      }
    } catch {
      // Nexus may not be running yet — start with empty list
    } finally {
      setLoading(false);
    }
  }

  // WebSocket subscription for live events
  let ws: WebSocket | null = null;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

  function connectWs() {
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    const url = `${proto}//${location.host}/ws`;
    try {
      ws = new WebSocket(url);

      ws.onopen = () => setWsConnected(true);

      ws.onmessage = (e) => {
        try {
          const msg = JSON.parse(e.data);
          if (msg.topic !== "events" || msg.event !== "tool_event") return;
          const ev = msg.data as ToolEvent;
          setEvents((prev) => [ev, ...prev].slice(0, 500));
        } catch { /* ignore */ }
      };

      ws.onclose = () => {
        setWsConnected(false);
        reconnectTimer = setTimeout(() => {
          reconnectTimer = null;
          connectWs();
        }, 5000);
      };

      ws.onerror = () => ws?.close();
    } catch { /* WebSocket unavailable */ }
  }

  onMount(() => {
    loadHistory();
    connectWs();
  });

  onCleanup(() => {
    if (reconnectTimer) clearTimeout(reconnectTimer);
    if (ws) { ws.onclose = null; ws.close(); }
  });

  return (
    <div class="flex flex-col h-full bg-gray-950 text-gray-200">
      {/* Header */}
      <div class="flex items-center gap-4 px-4 py-3 border-b border-gray-800">
        <h2 class="text-sm font-semibold text-gray-100">Activity</h2>
        <div
          class={`w-2 h-2 rounded-full ${wsConnected() ? "bg-green-500" : "bg-gray-600"}`}
          title={wsConnected() ? "Live" : "Disconnected"}
        />
        <span class="text-xs text-gray-500">{filtered().length} events</span>

        {/* Filters */}
        <input
          class="ml-auto bg-gray-900 border border-gray-700 rounded px-2 py-1 text-xs text-gray-300 placeholder-gray-600 w-36"
          placeholder="Session filter…"
          value={sessionFilter()}
          onInput={(e) => setSessionFilter(e.currentTarget.value)}
        />
        <input
          class="bg-gray-900 border border-gray-700 rounded px-2 py-1 text-xs text-gray-300 placeholder-gray-600 w-32"
          placeholder="Agent filter…"
          value={agentFilter()}
          onInput={(e) => setAgentFilter(e.currentTarget.value)}
        />
        <button
          class="text-xs text-gray-500 hover:text-gray-300 border border-gray-700 rounded px-2 py-1"
          onClick={loadHistory}
        >
          Refresh
        </button>
      </div>

      {/* Column headers */}
      <div class="flex items-center gap-3 px-4 py-1 border-b border-gray-800 text-xs text-gray-600 select-none">
        <span class="w-3" />
        <span class="w-28 shrink-0">Event</span>
        <span class="w-32 shrink-0">Tool</span>
        <span class="w-40 shrink-0">Model</span>
        <span class="w-16 shrink-0 text-right">Latency</span>
        <span class="w-20 shrink-0 text-right">Cost</span>
        <span class="w-24 shrink-0 text-right">Tokens</span>
        <span class="ml-auto shrink-0 text-right">Timestamp</span>
      </div>

      {/* Event list */}
      <div class="flex-1 overflow-y-auto">
        <Show
          when={!loading()}
          fallback={
            <div class="flex items-center justify-center h-32 text-gray-600 text-sm">
              Loading…
            </div>
          }
        >
          <Show
            when={filtered().length > 0}
            fallback={
              <div class="flex items-center justify-center h-32 text-gray-600 text-sm">
                No events yet. Run <code class="mx-1 font-mono text-gray-400">hex hook observe-pre/post</code> hooks to capture tool calls.
              </div>
            }
          >
            <For each={filtered()}>
              {(event) => <EventRow event={event} />}
            </For>
          </Show>
        </Show>
      </div>
    </div>
  );
};

export default ActivityPanel;
