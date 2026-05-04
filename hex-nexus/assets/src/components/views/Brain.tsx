/**
 * Brain.tsx — wp-brain-dashboard, full three-pane layout.
 *
 * Page layout:
 *   ┌─────────────────────────────────────────────────────────┐
 *   │  TEAM rail │  CENTER (Kanban + Decisions + Swarms +    │
 *   │  (left)    │           Health)         │  CHAT (right) │
 *   └─────────────────────────────────────────────────────────┘
 *   │  EVENT FEED (collapsible bottom strip)                 │
 *   └─────────────────────────────────────────────────────────┘
 *
 * Status of each pane:
 *   - TeamRail        : live (groups 25 personas by category, online dots from /api/hex-agents)
 *   - KanbanLanes     : live (projects /api/swarms/active tasks into 4 lanes)
 *   - DecisionsPanel  : live (reuses /api/decisions from M1)
 *   - SwarmsPanel     : live (/api/swarms/active)
 *   - HealthPanel     : live (/api/health + /api/sched/improver/status)
 *   - ChatPanel       : input shell only — full WebSocket dispatch lands in M3
 *   - EventFeed       : static placeholder — STDB subscription wiring lands later
 */
import { Component, For, Show, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { restClient } from "../../services/rest-client";

// ── Persona registry (mirrors hex-cli/assets/agents/hex/hex/) ────────────────
// Categories match the org-chart in the operator briefing. Order intentional.

interface Persona {
  name: string;
  category: "PRODUCT" | "ENGINEERING" | "QUALITY" | "DESIGN" | "OPS";
  color: string; // tailwind text-color class
}

const PERSONAS: Persona[] = [
  { name: "pm-agent",              category: "PRODUCT",     color: "text-purple-400" },
  { name: "planner",               category: "PRODUCT",     color: "text-blue-400" },
  { name: "feature-developer",     category: "PRODUCT",     color: "text-purple-400" },
  { name: "dependency-analyst",    category: "PRODUCT",     color: "text-cyan-400" },
  { name: "behavioral-spec-writer",category: "PRODUCT",     color: "text-green-400" },

  { name: "swarm-coordinator",     category: "ENGINEERING", color: "text-cyan-400" },
  { name: "hex-coder",             category: "ENGINEERING", color: "text-green-400" },
  { name: "hex-tester",            category: "ENGINEERING", color: "text-green-400" },
  { name: "hex-fixer",             category: "ENGINEERING", color: "text-orange-400" },
  { name: "hex-documenter",        category: "ENGINEERING", color: "text-yellow-400" },
  { name: "hex-ux",                category: "ENGINEERING", color: "text-pink-400" },
  { name: "rust-refactorer",       category: "ENGINEERING", color: "text-orange-400" },
  { name: "integrator",            category: "ENGINEERING", color: "text-yellow-400" },

  { name: "hex-reviewer",          category: "QUALITY",     color: "text-cyan-400" },
  { name: "validation-judge",      category: "QUALITY",     color: "text-red-400" },
  { name: "adversarial-red",       category: "QUALITY",     color: "text-red-400" },
  { name: "adversarial-blue",      category: "QUALITY",     color: "text-blue-400" },
  { name: "adr-reviewer",          category: "QUALITY",     color: "text-yellow-400" },
  { name: "dead-code-analyzer",    category: "QUALITY",     color: "text-orange-400" },
  { name: "scaffold-validator",    category: "QUALITY",     color: "text-yellow-400" },

  { name: "cli-designer",          category: "DESIGN",      color: "text-cyan-400" },
  { name: "ux-designer",           category: "DESIGN",      color: "text-pink-400" },

  { name: "dev-tracker",           category: "OPS",         color: "text-blue-400" },
  { name: "status-monitor",        category: "OPS",         color: "text-blue-400" },
];

// ── Types ────────────────────────────────────────────────────────────────────

interface SwarmTask { id: string; title: string; status: string; agentId?: string; agent_id?: string; }
interface Swarm { id: string; name?: string; status?: string; tasks?: SwarmTask[]; }
interface DecisionItem {
  id: string; kind: string;
  severity: "CRITICAL" | "HIGH" | "MEDIUM" | "LOW";
  title: string; reason: string; ageSeconds: number;
  suggestedAction: string; link: string | null;
}
interface DecisionsResponse { items: DecisionItem[]; total: number; bySeverity: Record<string, number>; }
interface ImproverStatus { score?: number; mean_reward?: number; meanReward?: number; topHypothesis?: string; deadLetter?: number; }

// ── Helpers ──────────────────────────────────────────────────────────────────

function severityClass(s: string): string {
  switch (s) {
    case "CRITICAL": return "bg-red-900/40 text-red-300 border-red-700";
    case "HIGH":     return "bg-orange-900/40 text-orange-300 border-orange-700";
    case "MEDIUM":   return "bg-yellow-900/30 text-yellow-300 border-yellow-700";
    default:         return "bg-gray-800 text-gray-400 border-gray-700";
  }
}

function ageShort(seconds: number): string {
  if (!seconds || seconds <= 0) return "—";
  if (seconds < 60) return `${seconds}s`;
  const m = Math.floor(seconds / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

function lane(status: string): "Backlog" | "Ready" | "Doing" | "Done" {
  switch (status) {
    case "in_progress": case "assigned": return "Doing";
    case "completed": case "done":       return "Done";
    case "blocked":                       return "Backlog";
    default:                              return "Ready";
  }
}

// ── Subcomponents ────────────────────────────────────────────────────────────

const TeamRail: Component<{ onlineNames: () => Set<string> }> = (props) => {
  const grouped = createMemo(() => {
    const cats: Record<string, Persona[]> = { PRODUCT: [], ENGINEERING: [], QUALITY: [], DESIGN: [], OPS: [] };
    for (const p of PERSONAS) cats[p.category].push(p);
    return cats;
  });

  return (
    <aside class="w-72 border-r border-gray-800 bg-gray-950 overflow-y-auto px-3 py-4">
      <h2 class="text-[11px] font-bold uppercase tracking-wider text-gray-500 mb-3 px-1">Team</h2>
      <For each={Object.entries(grouped())}>
        {([cat, members]) => (
          <div class="mb-5">
            <div class="flex items-center justify-between px-1 mb-1.5">
              <span class="text-[10px] font-bold uppercase tracking-wider text-gray-400">{cat}</span>
              <span class="text-[10px] text-gray-600">{members.length}</span>
            </div>
            <ul class="space-y-0.5">
              <For each={members}>
                {(p) => (
                  <li class="flex items-center gap-2 px-2 py-1 rounded hover:bg-gray-900 cursor-pointer text-xs">
                    <span
                      class={`h-1.5 w-1.5 rounded-full flex-shrink-0 ${
                        props.onlineNames().has(p.name) ? "bg-green-500" : "bg-gray-700"
                      }`}
                    />
                    <span class={`${p.color} truncate`}>{p.name}</span>
                  </li>
                )}
              </For>
            </ul>
          </div>
        )}
      </For>
    </aside>
  );
};

const KanbanLanes: Component<{ swarms: () => Swarm[] }> = (props) => {
  const tasksByLane = createMemo(() => {
    const lanes: Record<string, SwarmTask[]> = { Backlog: [], Ready: [], Doing: [], Done: [] };
    for (const s of props.swarms()) {
      for (const t of s.tasks || []) {
        const l = lane(t.status || "");
        // cap each lane to 8 items for visual sanity
        if (lanes[l].length < 8) lanes[l].push(t);
      }
    }
    return lanes;
  });

  return (
    <section class="bg-gray-900/50 border border-gray-800 rounded-lg p-3 mb-4">
      <h3 class="text-xs font-bold uppercase tracking-wider text-gray-400 mb-2 px-1">Kanban</h3>
      <div class="grid grid-cols-4 gap-2">
        <For each={["Backlog", "Ready", "Doing", "Done"]}>
          {(laneName) => (
            <div class="bg-gray-950 border border-gray-800 rounded p-2 min-h-[120px]">
              <div class="flex items-center justify-between mb-1.5">
                <span class="text-[10px] font-bold uppercase tracking-wider text-gray-500">{laneName}</span>
                <span class="text-[10px] text-gray-600">{tasksByLane()[laneName].length}</span>
              </div>
              <ul class="space-y-1">
                <For each={tasksByLane()[laneName]}>
                  {(t) => {
                    const agentId = t.agentId || t.agent_id || "";
                    const dot = agentId ? "●" : "○";
                    return (
                      <li
                        class="text-[11px] text-gray-300 bg-gray-900 border border-gray-800 rounded px-2 py-1 truncate"
                        title={t.title}
                      >
                        <span class="text-gray-500 mr-1">{dot}</span>
                        {t.title.slice(0, 28)}{t.title.length > 28 ? "…" : ""}
                      </li>
                    );
                  }}
                </For>
                <Show when={tasksByLane()[laneName].length === 0}>
                  <li class="text-[10px] text-gray-700 italic px-2 py-2">empty</li>
                </Show>
              </ul>
            </div>
          )}
        </For>
      </div>
    </section>
  );
};

const DecisionsPanel: Component<{ data: () => DecisionsResponse | null }> = (props) => (
  <section class="bg-gray-900/50 border border-gray-800 rounded-lg p-3 mb-4">
    <div class="flex items-center justify-between mb-2 px-1">
      <h3 class="text-xs font-bold uppercase tracking-wider text-gray-400">Decisions Needed</h3>
      <Show when={props.data()}>
        {(d) => (
          <div class="flex gap-1.5">
            <For each={["CRITICAL", "HIGH", "MEDIUM"]}>
              {(s) => (
                <Show when={(d().bySeverity[s] || 0) > 0}>
                  <span class={`text-[10px] font-bold px-1.5 py-0.5 rounded border ${severityClass(s)}`}>
                    {d().bySeverity[s]} {s[0]}
                  </span>
                </Show>
              )}
            </For>
          </div>
        )}
      </Show>
    </div>
    <Show
      when={(props.data()?.items || []).length > 0}
      fallback={
        <div class="text-center text-gray-600 text-xs py-3">
          ✓ caught up — no decisions pending
        </div>
      }
    >
      <ul class="space-y-1.5">
        <For each={(props.data()?.items || []).slice(0, 5)}>
          {(item) => (
            <li class="flex items-start gap-2 text-xs">
              <span class={`text-[10px] font-bold px-1.5 py-0.5 rounded border ${severityClass(item.severity)} flex-shrink-0`}>
                {item.severity[0]}
              </span>
              <span class="text-gray-300 truncate flex-1" title={item.title}>{item.title}</span>
              <span class="text-[10px] text-gray-600 flex-shrink-0">{ageShort(item.ageSeconds)}</span>
            </li>
          )}
        </For>
        <Show when={(props.data()?.items.length || 0) > 5}>
          <li class="text-[11px] text-gray-500 text-center pt-1">
            <a href="#/decisions" class="hover:text-gray-300 underline">
              view all {props.data()?.total} →
            </a>
          </li>
        </Show>
      </ul>
    </Show>
  </section>
);

const SwarmsPanel: Component<{ swarms: () => Swarm[] }> = (props) => (
  <section class="bg-gray-900/50 border border-gray-800 rounded-lg p-3 mb-4">
    <h3 class="text-xs font-bold uppercase tracking-wider text-gray-400 mb-2 px-1">Swarms</h3>
    <Show
      when={props.swarms().length > 0}
      fallback={<div class="text-xs text-gray-600 italic">no active swarms</div>}
    >
      <ul class="space-y-1.5">
        <For each={props.swarms().slice(0, 5)}>
          {(s) => {
            const tasks = s.tasks || [];
            const completed = tasks.filter((t) => t.status === "completed").length;
            const failed = tasks.filter((t) => t.status === "failed").length;
            return (
              <li class="text-xs flex items-center gap-2">
                <span class="h-1.5 w-1.5 rounded-full bg-green-500 flex-shrink-0" />
                <span class="text-gray-300 truncate flex-1" title={s.id}>{s.name || s.id.slice(0, 12)}</span>
                <span class="text-[10px] text-gray-500 flex-shrink-0">
                  {completed}/{tasks.length} · {failed} fail
                </span>
              </li>
            );
          }}
        </For>
      </ul>
    </Show>
  </section>
);

const HealthPanel: Component<{ improver: () => ImproverStatus | null; swarmCount: () => number }> = (props) => (
  <section class="bg-gray-900/50 border border-gray-800 rounded-lg p-3">
    <h3 class="text-xs font-bold uppercase tracking-wider text-gray-400 mb-2 px-1">Health</h3>
    <dl class="grid grid-cols-2 gap-y-1.5 gap-x-4 text-xs">
      <dt class="text-gray-500">Homeostasis</dt>
      <dd class="text-gray-200 font-mono text-right">
        {props.improver()?.score ?? "—"}
        <Show when={props.improver()?.score !== undefined && (props.improver()?.score ?? 0) > 50}>
          <span class="text-green-400 ml-1">↗</span>
        </Show>
      </dd>
      <dt class="text-gray-500">Q-reward</dt>
      <dd class="text-gray-200 font-mono text-right">
        {(() => {
          const r = props.improver()?.mean_reward ?? props.improver()?.meanReward;
          return r === undefined ? "—" : (r >= 0 ? "+" : "") + r.toFixed(3);
        })()}
      </dd>
      <dt class="text-gray-500">Active swarms</dt>
      <dd class="text-gray-200 font-mono text-right">{props.swarmCount()}</dd>
      <dt class="text-gray-500">Dead-letter</dt>
      <dd class="text-gray-200 font-mono text-right">{props.improver()?.deadLetter ?? "—"}</dd>
    </dl>
  </section>
);

interface ChatMessage {
  from: "you" | string; // "you" or persona name
  text: string;
  ts: string;
  model?: string;
  pending?: boolean;
  error?: boolean;
}

// Parse "@<role> <message>" — returns { role, message } or null if no @-mention.
function parseAtMention(text: string): { role: string; message: string } | null {
  const m = text.match(/^@([\w-]+)\s+([\s\S]+)$/);
  if (!m) return null;
  return { role: m[1], message: m[2].trim() };
}

const ChatPanel: Component = () => {
  const [input, setInput] = createSignal("");
  const [history, setHistory] = createSignal<ChatMessage[]>([
    {
      from: "system",
      text: "Type @<role> <message> and the agent's persona-driven response streams back here. Try `@pm-agent classify: add Redis adapter for caching` or `@cli-designer review hex sched --help`.",
      ts: new Date().toISOString(),
    },
  ]);
  const [showSuggestions, setShowSuggestions] = createSignal(false);
  const [suggestionQuery, setSuggestionQuery] = createSignal("");

  // Filter personas for @-mention autocomplete.
  const suggestions = createMemo(() => {
    const q = suggestionQuery().toLowerCase();
    return PERSONAS.filter((p) => p.name.toLowerCase().startsWith(q)).slice(0, 8);
  });

  const handleInput = (val: string) => {
    setInput(val);
    // Show suggestions while the user is mid-@-mention (cursor right after @<chars>).
    const m = val.match(/(?:^|\s)@([\w-]*)$/);
    if (m) {
      setSuggestionQuery(m[1]);
      setShowSuggestions(true);
    } else {
      setShowSuggestions(false);
    }
  };

  const completeSuggestion = (name: string) => {
    const cur = input();
    const replaced = cur.replace(/(?:^|\s)@([\w-]*)$/, (m, _q, _o, _s) => {
      // Preserve the leading whitespace if any.
      const leadingSpace = m.startsWith(" ") || m.startsWith("\n") ? m[0] : "";
      return `${leadingSpace}@${name} `;
    });
    setInput(replaced);
    setShowSuggestions(false);
  };

  const handleSend = async () => {
    const text = input().trim();
    if (!text) return;
    const parsed = parseAtMention(text);
    const ts = new Date().toISOString();
    setHistory((h) => [...h, { from: "you", text, ts }]);
    setInput("");
    setShowSuggestions(false);

    if (!parsed) {
      setHistory((h) => [
        ...h,
        {
          from: "system",
          text: "Start your message with @<role> — e.g. `@pm-agent classify ...`. Without an @-mention there's no agent to dispatch to.",
          ts: new Date().toISOString(),
          error: true,
        },
      ]);
      return;
    }

    // Optimistic pending bubble.
    const pendingId = Math.random().toString(36).slice(2);
    setHistory((h) => [
      ...h,
      { from: parsed.role, text: "thinking...", ts: new Date().toISOString(), pending: true, model: pendingId },
    ]);

    try {
      const resp = await restClient.post<{ role: string; model: string; content: string }>(
        "/api/brain/chat",
        { role: parsed.role, message: parsed.message },
      );
      // Replace the pending bubble with the real response.
      setHistory((h) => h.map((m) =>
        m.pending && m.model === pendingId
          ? { from: resp.role, text: resp.content || "(empty response)", ts: new Date().toISOString(), model: resp.model }
          : m,
      ));
    } catch (e) {
      const err = e instanceof Error ? e.message : String(e);
      setHistory((h) => h.map((m) =>
        m.pending && m.model === pendingId
          ? { from: parsed.role, text: `dispatch failed: ${err}`, ts: new Date().toISOString(), error: true }
          : m,
      ));
    }
  };

  const personaColor = (name: string): string => {
    const p = PERSONAS.find((x) => x.name === name);
    return p?.color ?? "text-gray-300";
  };

  return (
    <aside class="w-96 border-l border-gray-800 bg-gray-950 flex flex-col">
      <header class="px-3 py-2 border-b border-gray-800">
        <h2 class="text-[11px] font-bold uppercase tracking-wider text-gray-500">Chat</h2>
      </header>
      <div class="flex-1 overflow-y-auto px-3 py-3 space-y-3">
        <For each={history()}>
          {(msg) => (
            <div class="text-xs">
              <div class="text-[10px] mb-0.5 flex items-center gap-2">
                <span class={msg.from === "you" ? "text-gray-400" : personaColor(msg.from)}>
                  {msg.from === "you" ? "you" : `@${msg.from}`}
                </span>
                <span class="text-gray-600">· {new Date(msg.ts).toLocaleTimeString()}</span>
                <Show when={msg.model && !msg.pending}>
                  <span class="text-[9px] text-gray-700 ml-auto truncate max-w-[120px]" title={msg.model}>
                    {msg.model}
                  </span>
                </Show>
              </div>
              <div
                class={`leading-relaxed whitespace-pre-wrap ${
                  msg.error ? "text-red-400" : msg.pending ? "text-gray-500 italic" : "text-gray-300"
                }`}
              >
                {msg.text}
              </div>
            </div>
          )}
        </For>
      </div>
      <footer class="border-t border-gray-800 p-2 relative">
        <Show when={showSuggestions() && suggestions().length > 0}>
          <ul class="absolute bottom-full left-2 right-2 mb-1 bg-gray-900 border border-gray-700 rounded shadow-xl max-h-48 overflow-y-auto z-10">
            <For each={suggestions()}>
              {(p) => (
                <li
                  class="px-2 py-1 text-xs hover:bg-gray-800 cursor-pointer flex items-center gap-2"
                  onClick={() => completeSuggestion(p.name)}
                >
                  <span class="text-[10px] text-gray-600 w-16">{p.category}</span>
                  <span class={p.color}>{p.name}</span>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <textarea
          value={input()}
          onInput={(e) => handleInput(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              handleSend();
            } else if (e.key === "Escape") {
              setShowSuggestions(false);
            }
          }}
          placeholder="@pm-agent classify..."
          rows={3}
          class="w-full bg-gray-900 border border-gray-800 rounded px-2 py-1.5 text-xs text-gray-200 placeholder-gray-600 focus:outline-none focus:border-cyan-700 resize-none"
        />
        <div class="flex items-center justify-between mt-1.5">
          <span class="text-[10px] text-gray-600">@ to mention · Cmd+Enter to send</span>
          <button
            onClick={handleSend}
            disabled={!input().trim()}
            class="px-2 py-1 text-[11px] font-medium bg-cyan-900/30 text-cyan-300 border border-cyan-700 rounded hover:bg-cyan-900/50 transition disabled:opacity-30 disabled:cursor-not-allowed"
          >
            Send
          </button>
        </div>
      </footer>
    </aside>
  );
};

interface RawEvent {
  id?: string | number;
  type?: string;
  topic?: string;
  source?: string;
  message?: string;
  text?: string;
  payload?: any;
  ts?: string;
  timestamp?: string;
  created_at?: string;
}

const EventFeed: Component = () => {
  const [events, setEvents] = createSignal<RawEvent[]>([]);

  const fetchEvents = async () => {
    try {
      const resp = await restClient.get<RawEvent[] | { events: RawEvent[] }>("/api/events?limit=20");
      const arr = Array.isArray(resp) ? resp : (resp as any).events || [];
      setEvents(arr.slice(-12).reverse()); // newest first, last 12
    } catch { /* ignore */ }
  };

  let pollHandle: number | undefined;
  onMount(() => {
    fetchEvents();
    pollHandle = window.setInterval(fetchEvents, 5000);
  });
  onCleanup(() => { if (pollHandle !== undefined) window.clearInterval(pollHandle); });

  const renderEvent = (e: RawEvent): { time: string; source: string; text: string } => {
    const ts = e.ts || e.timestamp || e.created_at || "";
    const time = ts ? new Date(ts).toLocaleTimeString().slice(0, 8) : "—";
    const source = e.source || e.type || e.topic || "·";
    const text = e.message || e.text || (e.payload ? JSON.stringify(e.payload).slice(0, 80) : "");
    return { time, source, text };
  };

  return (
    <footer class="border-t border-gray-800 bg-gray-950 px-4 py-1.5 flex items-center gap-3 text-[11px] text-gray-500 overflow-x-auto whitespace-nowrap shrink-0">
      <span class="text-[10px] font-bold uppercase tracking-wider text-gray-600 shrink-0">Events</span>
      <Show
        when={events().length > 0}
        fallback={<span class="italic text-gray-700">no recent events</span>}
      >
        <For each={events()}>
          {(e) => {
            const r = renderEvent(e);
            return (
              <span class="shrink-0">
                <span class="text-gray-700">{r.time}</span>
                <span class="ml-1.5 text-cyan-500">{r.source}</span>
                <span class="ml-1.5 text-gray-400">{r.text.slice(0, 80)}</span>
                <span class="text-gray-800 mx-2">·</span>
              </span>
            );
          }}
        </For>
      </Show>
    </footer>
  );
};

// ── Main page ────────────────────────────────────────────────────────────────

const Brain: Component = () => {
  const [swarms, setSwarms] = createSignal<Swarm[]>([]);
  const [decisions, setDecisions] = createSignal<DecisionsResponse | null>(null);
  const [improver, setImprover] = createSignal<ImproverStatus | null>(null);
  const [agents, setAgents] = createSignal<{ name?: string; capabilities?: { role?: string } }[]>([]);

  const onlineNames = createMemo(() => {
    const set = new Set<string>();
    for (const a of agents()) {
      const role = a.capabilities?.role;
      if (role) set.add(role);
      if (a.name) {
        // names are like "pm-agent-bazzite.lan" — extract the role prefix
        for (const p of PERSONAS) {
          if (a.name.startsWith(p.name)) { set.add(p.name); break; }
        }
      }
    }
    return set;
  });

  const refresh = async () => {
    try {
      const s = await restClient.get<Swarm[]>("/api/swarms/active");
      setSwarms(Array.isArray(s) ? s : []);
    } catch { /* nexus may be down */ }
    try {
      const d = await restClient.get<DecisionsResponse>("/api/decisions");
      setDecisions(d);
    } catch { /* ignore */ }
    try {
      const i = await restClient.get<{ agents: any[] }>("/api/hex-agents");
      setAgents(i.agents || []);
    } catch { /* ignore */ }
    // improver status — best-effort, not all setups expose this REST surface.
    try {
      const im = await restClient.get<ImproverStatus>("/api/sched/improver/status");
      setImprover(im);
    } catch { /* improver not exposed via REST in all builds */ }
  };

  let pollHandle: number | undefined;
  onMount(() => {
    refresh();
    pollHandle = window.setInterval(refresh, 15000);
  });
  onCleanup(() => { if (pollHandle !== undefined) window.clearInterval(pollHandle); });

  return (
    <div class="flex flex-col h-screen bg-gray-950 text-gray-100">
      {/* Top bar */}
      <header class="px-4 py-2 border-b border-gray-800 flex items-center gap-3 shrink-0">
        <h1 class="text-sm font-bold tracking-wide text-gray-200">HEX BRAIN</h1>
        <span class="text-[11px] text-gray-500">
          {PERSONAS.length} personas · {swarms().length} active swarms · {decisions()?.total ?? 0} decisions
        </span>
        <span class="ml-auto text-[10px] text-gray-600">refreshes every 15s</span>
      </header>

      {/* Three-pane main */}
      <main class="flex flex-1 overflow-hidden">
        <TeamRail onlineNames={onlineNames} />

        <div class="flex-1 overflow-y-auto px-4 py-4">
          <KanbanLanes swarms={swarms} />
          <DecisionsPanel data={decisions} />
          <SwarmsPanel swarms={swarms} />
          <HealthPanel improver={improver} swarmCount={() => swarms().length} />
        </div>

        <ChatPanel />
      </main>

      <EventFeed />
    </div>
  );
};

export default Brain;
