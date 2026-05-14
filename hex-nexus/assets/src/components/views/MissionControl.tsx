/**
 * MissionControl.tsx — hex-native operator console.
 *
 * NOT a Hermes clone. Built around hex's actual primitives:
 *   - factory roster (c-suite + IC personas with open-work counts)
 *   - live activity stream (twin verdicts + executor + autonomous commits
 *     interleaved chronologically — what the factory IS doing right now)
 *   - attention items (compact, in the rail; the wedge surface)
 *   - operator compose at the top — natural-language intent fired through
 *     `hex agent run` typed-tool loop
 *
 * One page. No drill-down footer. No tabbed history. Shows the system
 * doing its work.
 */

import { Component, For, Show, createSignal, onMount, onCleanup, createMemo } from "solid-js";
import { restClient } from "../../services/rest-client";

interface PersonaRow {
  role: string;
  display_name: string;
  paused: boolean;
  last_tick_at: string;
}

interface ExecutedRow {
  id: number;
  kind: string;
  path: string | null;
  success: boolean;
  error: string;
  executed_at: string;
  evidence: string;
}

interface ActionRow {
  id: number;
  kind: string;
  proposed_by: string;
  status: string;
  twin_verdict: string;
  twin_rationale: string;
  escalate_reason: string;
}

interface CommitmentRow {
  id: number;
  role: string;
  action: string;
  success_artifact: string;
  status: string;
  created_at: string;
}

interface AttentionItem {
  id: string;
  priority: 0 | 1 | 2;
  kind: string;
  title: string;
  subtitle: string;
  age_seconds: number;
  action_url?: string;
  cli_repro?: string;
}

interface LiveEvent {
  id: number;
  event_type: string;
  created_at: string;
  session_id: string;
  preview: string;
}

interface ChatMessage {
  msg_id: number;
  from_role: string;
  to_role: string;
  message: string;
  created_at: string;
}

interface Payload {
  stdb_alive: boolean;
  pulse?: { autonomous_commits_today?: number };
  personas: PersonaRow[];
  activity: { recent_executed: ExecutedRow[]; open_merge_requests: any[] };
  pending_decisions: { actions: ActionRow[]; commitments: CommitmentRow[]; anomalies: any[] };
  attention_feed?: AttentionItem[];
  live_events?: LiveEvent[];
  recent_messages?: ChatMessage[];
}

const REFRESH_MS = 5000;

const ageSinceIso = (iso: string): string => {
  if (!iso) return "—";
  const t = Date.parse(iso);
  if (isNaN(t)) return "—";
  return ageSec(Math.max(0, Math.floor((Date.now() - t) / 1000)));
};

const ageSec = (s: number): string => {
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
};

// STDB occasionally hands us non-ISO timestamps wrapped in a Debug
// representation. Normalize so Date.parse works.
const tsToEpoch = (raw: any): number => {
  if (!raw) return 0;
  if (typeof raw === "number") return raw;
  const s = String(raw);
  const m = s.match(/__timestamp_micros_since_unix_epoch__:\s*(\d+)/);
  if (m) return Math.floor(parseInt(m[1], 10) / 1000);
  const t = Date.parse(s);
  return isNaN(t) ? 0 : t;
};

const ageSinceAny = (raw: any): string => {
  const ms = tsToEpoch(raw);
  if (!ms) return "—";
  return ageSec(Math.max(0, Math.floor((Date.now() - ms) / 1000)));
};

const truncate = (s: string, n: number): string =>
  s.length > n ? s.slice(0, n) + "…" : s;

const MODELS = [
  { id: "anthropic/claude-haiku-4.5", label: "haiku" },
  { id: "anthropic/claude-sonnet-4-6", label: "sonnet" },
  { id: "anthropic/claude-opus-4-7", label: "opus" },
];

const MissionControl: Component = () => {
  const [data, setData] = createSignal<Payload | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [intent, setIntent] = createSignal("");
  const [model, setModel] = createSignal(MODELS[0].id);
  const [maxIter, setMaxIter] = createSignal(6);
  const [running, setRunning] = createSignal(false);
  const [lastDispatch, setLastDispatch] = createSignal<string>("");

  let timer: ReturnType<typeof setInterval> | null = null;
  const refresh = async () => {
    try {
      const d = await restClient.get("/api/mission-control");
      setData(d);
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
  onCleanup(() => { if (timer) clearInterval(timer); });

  // Two modes — natural-language intent dispatches `hex agent run`;
  // a leading @role sends to the persona board (chat with a persona).
  // Example:
  //   "use code_patch to ..."   → POST /api/agent/run
  //   "@cto draft a security ADR" → POST /api/org/send-message to=cto
  //   "@all weekly sync"        → broadcast to c-suite
  const dispatch = async () => {
    const text = intent().trim();
    if (!text || running()) return;
    const chatMatch = text.match(/^@(\S+)\s+([\s\S]+)$/);
    setRunning(true);
    if (chatMatch) {
      const [, target, msg] = chatMatch;
      setLastDispatch(`→ chat to @${target}: routing…`);
      try {
        const from = target === "all" ? "operator" : "operator";
        const body: any = { from, content: msg };
        if (target !== "all") body.to = target;
        const resp = await restClient.post("/api/org/send-message", body);
        const routed = resp?.routed_to || [];
        setLastDispatch(`✓ chat routed → ${Array.isArray(routed) && routed.length ? routed.join(", ") : target}`);
        setIntent("");
        setTimeout(refresh, 1500);
      } catch (e: any) {
        setLastDispatch(`✗ chat error: ${e?.message || String(e)}`);
      } finally {
        setRunning(false);
        setTimeout(() => setLastDispatch(""), 12000);
      }
      return;
    }
    setLastDispatch("dispatching agent run…");
    try {
      const resp = await restClient.post("/api/agent/run", {
        intent: text,
        max_iterations: maxIter(),
        model: model(),
      });
      const steps = (resp?.steps || []).length;
      setLastDispatch(
        `✓ ${resp?.iterations ?? 0} iter · ${steps} step${steps === 1 ? "" : "s"} · ${resp?.stop_reason ?? "?"} · ${resp?.elapsed_ms ?? 0}ms`
      );
      setIntent("");
      await refresh();
    } catch (e: any) {
      setLastDispatch(`✗ ${e?.message || String(e)}`);
    } finally {
      setRunning(false);
      setTimeout(() => setLastDispatch(""), 12000);
    }
  };

  // ── Derived: per-persona status ──────────────────────────────────
  // Translates raw counts into operator-readable status:
  //   "idle · 2m"                  (no open work)
  //   "drafting 2 actions"         (open work, no escalations)
  //   "blocked · 3 escalated"      (needs operator attention)
  //   "paused"                     (operator-suspended)
  interface FactoryRow {
    role: string;
    display_name: string;
    paused: boolean;
    last_tick_at: string;
    open: number;
    escalated: number;
    statusLine: string;
    statusColor: string;
  }
  const factoryRows = createMemo<FactoryRow[]>(() => {
    const d = data();
    if (!d) return [];
    const openByRole = new Map<string, number>();
    const escByRole = new Map<string, number>();
    for (const a of d.pending_decisions?.actions || []) {
      const by = a.proposed_by || "";
      if (by && !by.includes(":") && by !== "operator-passthrough") {
        openByRole.set(by, (openByRole.get(by) || 0) + 1);
        if (a.status === "escalated") escByRole.set(by, (escByRole.get(by) || 0) + 1);
      }
    }
    for (const c of d.pending_decisions?.commitments || []) {
      if (c.role && c.status !== "satisfied") {
        openByRole.set(c.role, (openByRole.get(c.role) || 0) + 1);
        if (c.status === "overdue" || c.status === "escalated")
          escByRole.set(c.role, (escByRole.get(c.role) || 0) + 1);
      }
    }
    return (d.personas || []).map((p) => {
      const open = openByRole.get(p.role) || 0;
      const esc = escByRole.get(p.role) || 0;
      const age = ageSinceAny(p.last_tick_at);
      let statusLine: string;
      let statusColor: string;
      if (p.paused) {
        statusLine = "paused";
        statusColor = "text-yellow-400";
      } else if (esc > 0) {
        statusLine = `${esc} blocked — needs you`;
        statusColor = "text-red-400";
      } else if (open > 0) {
        statusLine = `working on ${open} action${open === 1 ? "" : "s"}`;
        statusColor = "text-cyan-300";
      } else {
        statusLine = `idle · last tick ${age}`;
        statusColor = "text-zinc-500";
      }
      return { ...p, open, escalated: esc, statusLine, statusColor };
    });
  });

  const [selectedRole, setSelectedRole] = createSignal<string | null>(null);
  const [expandedAttention, setExpandedAttention] = createSignal<Set<string>>(new Set());
  const [actionBusy, setActionBusy] = createSignal<string | null>(null);
  const [actionStatus, setActionStatus] = createSignal<Record<string, string>>({});

  const abandonAttention = async (id: string, actionId?: number) => {
    if (!actionId) return;
    setActionBusy(id);
    try {
      const r = await restClient.post(`/v1/database/hex/call/proposed_action_close`, [actionId, "abandoned", "operator abandoned via dashboard"]).catch(() => null);
      setActionStatus({ ...actionStatus(), [id]: r ? "abandoned" : "tried abandon; check log" });
    } catch (e: any) {
      setActionStatus({ ...actionStatus(), [id]: `error: ${e?.message || e}` });
    } finally {
      setActionBusy(null);
      await refresh();
    }
  };

  const inspectAttention = (item: AttentionItem) => {
    const e = new Set(expandedAttention());
    if (e.has(item.id)) e.delete(item.id); else e.add(item.id);
    setExpandedAttention(e);
  };

  const copyCli = (cli?: string) => {
    if (!cli) return;
    navigator.clipboard?.writeText(cli);
  };

  // ── Derived: unified activity stream as conversational sentences ─
  // Each item reads like "actor verbed object" — operator can scan
  // the stream like a newsfeed rather than parsing log lines.
  interface ActivityItem {
    ts: number;
    icon: string;
    color: string;
    actor: string;          // who did it (cyan/purple/green text)
    actorColor: string;
    verb: string;           // past-tense action
    target: string;         // what they acted on
    detail?: string;        // optional secondary context (italic)
    role?: string;          // for filtering
    sourceId: string | number;
  }

  const activity = createMemo<ActivityItem[]>(() => {
    const d = data();
    if (!d) return [];
    const items: ActivityItem[] = [];
    const role = selectedRole();
    for (const ex of d.activity?.recent_executed || []) {
      const ts = tsToEpoch(ex.executed_at);
      if (!ts) continue;
      // Parse "auto-executed by ceo-twin: wrote /path (N bytes)" to
      // attribute back to the persona/twin actor.
      const m = ex.evidence?.match(/by (\S+):/);
      const actor = m ? m[1] : "executor";
      const path = ex.path || "(unknown)";
      const filename = path.split("/").pop() || path;
      items.push({
        ts,
        icon: ex.success ? "✎" : "✗",
        color: ex.success ? "text-cyan-300" : "text-red-400",
        actor,
        actorColor: actorColorFor(actor),
        verb: ex.success ? "wrote" : "tried to write",
        target: filename,
        detail: ex.success ? `${path} · action #${ex.id}` : (ex.error || `action #${ex.id}`),
        sourceId: ex.id,
      });
    }
    for (const ev of d.live_events || []) {
      const ts = tsToEpoch(ev.created_at);
      if (!ts) continue;
      // Heartbeats (brain_tick, improver_tick) are scheduler pulses, not
      // operator-meaningful events. Surface them via the loop-health
      // indicator below the header instead of cluttering the stream.
      if (ev.event_type === "brain_tick" || ev.event_type === "improver_tick") continue;
      const info = eventDecorate(ev.event_type);
      items.push({
        ts,
        icon: info.icon,
        color: info.color,
        actor: info.actor,
        actorColor: actorColorFor(info.actor),
        verb: info.verb,
        target: info.target || ev.event_type,
        detail: ev.preview ? humanizePreview(ev.event_type, ev.preview) : undefined,
        sourceId: `ev-${ev.id}`,
      });
    }
    items.sort((a, b) => b.ts - a.ts);
    const filtered = role ? items.filter((i) => i.actor === role || i.actor === `${role}-twin`) : items;
    return filtered.slice(0, 50);
  });

  // Loop health — track when each background loop last ticked so we
  // know they're alive without spamming the activity stream.
  const loopHealth = createMemo(() => {
    const evs = data()?.live_events || [];
    let brainTs = 0;
    let improverTs = 0;
    for (const ev of evs) {
      const ts = tsToEpoch(ev.created_at);
      if (ev.event_type === "brain_tick" && ts > brainTs) brainTs = ts;
      if (ev.event_type === "improver_tick" && ts > improverTs) improverTs = ts;
    }
    return { brainTs, improverTs };
  });

  const [mainTab, setMainTab] = createSignal<"activity" | "chat">("activity");

  const chatMessages = createMemo<ChatMessage[]>(() => {
    const msgs = data()?.recent_messages || [];
    const role = selectedRole();
    const filtered = role
      ? msgs.filter((m) => m.from_role === role || m.to_role === role)
      : msgs;
    // Oldest first so chat reads top-to-bottom
    return [...filtered].sort(
      (a, b) => tsToEpoch(a.created_at) - tsToEpoch(b.created_at)
    );
  });

  const attention = createMemo(() => data()?.attention_feed || []);
  const p0 = () => attention().filter((i) => i.priority === 0).length;
  const p1 = () => attention().filter((i) => i.priority === 1).length;

  return (
    <div class="flex flex-col h-screen bg-zinc-950 text-zinc-100 font-sans">
      {/* ─── Header ─── */}
      <header class="px-6 py-3 border-b border-zinc-800 flex items-center justify-between flex-wrap gap-3">
        <div class="flex items-baseline gap-3">
          <h1 class="text-base font-semibold tracking-tight">hex</h1>
          <span class="text-[11px] text-zinc-500 font-mono">operator console</span>
        </div>
        <div class="flex items-center gap-3 text-[11px]">
          <span class={data()?.stdb_alive ? "text-green-400" : "text-red-400"}>
            STDB {data()?.stdb_alive ? "✓" : "✗"}
          </span>
          <span class="text-zinc-500">·</span>
          <span
            class="text-zinc-500"
            title={
              `brain: ${loopHealth().brainTs ? `${ageSec(Math.max(0, Math.floor((Date.now() - loopHealth().brainTs) / 1000)))} ago` : "silent"}\n` +
              `improver: ${loopHealth().improverTs ? `${ageSec(Math.max(0, Math.floor((Date.now() - loopHealth().improverTs) / 1000)))} ago` : "silent"}`
            }
          >
            loops {loopHealth().brainTs && loopHealth().improverTs ? "✓" : "?"}
          </span>
          <span class="text-zinc-500">·</span>
          <span class="text-cyan-300 tabular-nums">
            {data()?.pulse?.autonomous_commits_today ?? "—"} commits today
          </span>
          <span class="text-zinc-500">·</span>
          <span class={p0() > 0 ? "text-red-400" : "text-zinc-500"}>{p0()} P0</span>
          <span class={p1() > 0 ? "text-amber-400" : "text-zinc-500"}>{p1()} P1</span>
          <span class="text-zinc-500">· refresh {REFRESH_MS / 1000}s</span>
        </div>
      </header>

      <Show when={error()}>
        <div class="px-6 py-2 bg-red-950/40 border-b border-red-900 text-red-300 text-xs">
          {error()}
        </div>
      </Show>

      {/* ─── Operator input (chat-or-dispatch) ─── */}
      <div class="px-6 py-3 border-b border-zinc-800 bg-zinc-900/40">
        <div class="flex items-center gap-2 mb-1.5 text-[11px]">
          <span class="text-zinc-500">
            <span class="text-cyan-300 font-mono">@role</span> chats with a persona ·
            <span class="text-zinc-300"> plain text</span> dispatches an agent run
          </span>
          <span class="ml-auto flex items-center gap-2 text-zinc-500">
            <select
              class="bg-zinc-900 border border-zinc-700 rounded px-1.5 py-0.5 text-zinc-300"
              value={model()}
              onChange={(e) => setModel(e.currentTarget.value)}
              disabled={running()}
            >
              <For each={MODELS}>{(m) => <option value={m.id}>{m.label}</option>}</For>
            </select>
            <label class="flex items-center gap-1">
              <span>iter</span>
              <input
                type="number"
                min="1"
                max="20"
                class="w-10 bg-zinc-900 border border-zinc-700 rounded px-1 py-0.5 tabular-nums text-zinc-300"
                value={maxIter()}
                onInput={(e) => setMaxIter(Math.max(1, Math.min(20, parseInt(e.currentTarget.value) || 6)))}
                disabled={running()}
              />
            </label>
            <span class="text-zinc-600">⌘↵</span>
          </span>
        </div>
        <div class="flex gap-2">
          <input
            class="flex-1 bg-zinc-950 border border-zinc-700 focus:border-cyan-600 focus:outline-none rounded px-3 py-2 text-sm font-mono"
            placeholder={intent().startsWith("@") ? "chat message → press ⌘↵ to send" : '"@cto draft an ADR" or "use code_patch to create docs/foo.md..."'}
            value={intent()}
            onInput={(e) => setIntent(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                e.preventDefault();
                dispatch();
              }
            }}
            disabled={running()}
          />
          <button
            class="px-5 rounded text-white text-sm disabled:opacity-50"
            classList={{
              "bg-cyan-700 hover:bg-cyan-600": !intent().startsWith("@"),
              "bg-purple-700 hover:bg-purple-600": intent().startsWith("@"),
            }}
            disabled={!intent().trim() || running()}
            onClick={dispatch}
          >
            {running() ? "Running…" : (intent().startsWith("@") ? "Send" : "Run")}
          </button>
        </div>
        <Show when={lastDispatch()}>
          <div class="text-[11px] text-zinc-400 mt-1.5 font-mono">{lastDispatch()}</div>
        </Show>
      </div>

      {/* ─── Two-column body ─── */}
      <div class="flex-1 grid grid-cols-12 gap-0 overflow-hidden">
        {/* Left rail: factory + attention */}
        <aside class="col-span-4 lg:col-span-3 border-r border-zinc-800 overflow-y-auto">
          <div class="px-4 py-3 border-b border-zinc-800">
            <div class="flex items-center justify-between mb-2">
              <h2 class="text-[10px] uppercase tracking-wide text-zinc-500">Factory</h2>
              <Show when={selectedRole()}>
                <button class="text-[10px] text-cyan-400 hover:underline" onClick={() => setSelectedRole(null)}>
                  clear filter
                </button>
              </Show>
            </div>
            <div class="space-y-1">
              <For each={factoryRows()}>{(p) => (
                <button
                  class="w-full text-left rounded px-2 py-1.5 border transition-colors"
                  classList={{
                    "border-cyan-700 bg-cyan-900/20": selectedRole() === p.role,
                    "border-transparent hover:bg-zinc-900": selectedRole() !== p.role,
                  }}
                  onClick={() => setSelectedRole(selectedRole() === p.role ? null : p.role)}
                  title={selectedRole() === p.role ? "click to clear filter" : `click to filter activity to ${p.role}`}
                >
                  <div class="flex items-center gap-2">
                    <span class={p.paused ? "text-yellow-400" : (p.escalated > 0 ? "text-red-400" : "text-green-400")}>●</span>
                    <span class="font-mono text-zinc-200 text-xs">{p.role}</span>
                  </div>
                  <div class={`text-[11px] ml-4 ${p.statusColor}`}>{p.statusLine}</div>
                </button>
              )}</For>
              <Show when={factoryRows().length === 0 && !loading()}>
                <div class="text-zinc-500 text-xs italic">No personas registered.</div>
              </Show>
            </div>
          </div>

          <div class="px-4 py-3">
            <div class="flex items-center justify-between mb-2">
              <h2 class="text-[10px] uppercase tracking-wide text-zinc-500">Attention</h2>
              <span class="text-[10px] text-zinc-500 tabular-nums">{attention().length}</span>
            </div>
            <Show
              when={attention().length > 0}
              fallback={<div class="text-zinc-500 text-xs italic">Nothing waiting.</div>}
            >
              <div class="space-y-1.5">
                <For each={attention().slice(0, 20)}>{(item) => {
                  const isOpen = () => expandedAttention().has(item.id);
                  const actionMatch = item.id.match(/^(escalation|commitment|autocommit|merge|anomaly)-(.+)$/);
                  const actionId = actionMatch && /^\d+$/.test(actionMatch[2]) ? parseInt(actionMatch[2], 10) : undefined;
                  return (
                    <div class="text-xs">
                      <button
                        class="w-full text-left rounded border transition-colors px-2 py-1.5 hover:bg-zinc-900"
                        classList={{
                          "border-red-800 bg-red-950/20": item.priority === 0 && !isOpen(),
                          "border-amber-800 bg-amber-950/10": item.priority === 1 && !isOpen(),
                          "border-zinc-800": item.priority === 2 && !isOpen(),
                          "border-cyan-700 bg-zinc-900": isOpen(),
                        }}
                        onClick={() => inspectAttention(item)}
                      >
                        <div class="flex items-baseline gap-1.5">
                          <span class={
                            item.priority === 0 ? "text-red-400" :
                            item.priority === 1 ? "text-amber-400" : "text-blue-400"
                          }>●</span>
                          <span class="text-zinc-200 truncate flex-1">{item.title}</span>
                          <span class="text-zinc-500 tabular-nums shrink-0 text-[10px]">{ageSec(item.age_seconds)}</span>
                          <span class="text-zinc-600 shrink-0">{isOpen() ? "▾" : "▸"}</span>
                        </div>
                      </button>
                      <Show when={isOpen()}>
                        <div class="border-l border-zinc-700 ml-2 mt-1 px-2 py-1.5 space-y-1.5">
                          <div class="text-[11px] text-zinc-400">{item.subtitle}</div>
                          <div class="flex flex-wrap gap-1.5">
                            <Show when={item.cli_repro}>
                              <button
                                class="px-2 py-0.5 rounded bg-zinc-800 hover:bg-zinc-700 text-zinc-200 text-[11px]"
                                onClick={() => copyCli(item.cli_repro)}
                                title={item.cli_repro}
                              >
                                Copy CLI
                              </button>
                            </Show>
                            <Show when={actionId !== undefined}>
                              <button
                                class="px-2 py-0.5 rounded bg-red-900/40 hover:bg-red-900 border border-red-800 text-red-200 text-[11px] disabled:opacity-50"
                                disabled={actionBusy() === item.id}
                                onClick={() => abandonAttention(item.id, actionId)}
                              >
                                {actionBusy() === item.id ? "…" : "Abandon"}
                              </button>
                            </Show>
                            <Show when={item.action_url}>
                              <a
                                class="px-2 py-0.5 rounded border border-zinc-700 hover:bg-zinc-800 text-zinc-200 text-[11px]"
                                href={item.action_url}
                              >
                                Inspect
                              </a>
                            </Show>
                          </div>
                          <Show when={actionStatus()[item.id]}>
                            <div class="text-[10px] text-zinc-500 italic">{actionStatus()[item.id]}</div>
                          </Show>
                        </div>
                      </Show>
                    </div>
                  );
                }}</For>
                <Show when={attention().length > 20}>
                  <div class="text-[10px] text-zinc-500 italic pt-1">
                    … {attention().length - 20} more
                  </div>
                </Show>
              </div>
            </Show>
          </div>
        </aside>

        {/* Main: Activity stream OR Chat — toggled */}
        <main class="col-span-8 lg:col-span-9 overflow-y-auto flex flex-col">
          <div class="px-6 py-2 border-b border-zinc-800 sticky top-0 bg-zinc-950 z-10 flex items-center gap-4">
            <button
              class="text-[11px] uppercase tracking-wide pb-1 border-b-2"
              classList={{
                "text-zinc-100 border-cyan-500": mainTab() === "activity",
                "text-zinc-500 border-transparent hover:text-zinc-300": mainTab() !== "activity",
              }}
              onClick={() => setMainTab("activity")}
            >
              Activity <span class="text-zinc-600">({activity().length})</span>
            </button>
            <button
              class="text-[11px] uppercase tracking-wide pb-1 border-b-2"
              classList={{
                "text-zinc-100 border-cyan-500": mainTab() === "chat",
                "text-zinc-500 border-transparent hover:text-zinc-300": mainTab() !== "chat",
              }}
              onClick={() => setMainTab("chat")}
            >
              Chat <span class="text-zinc-600">({chatMessages().length})</span>
            </button>
            <Show when={selectedRole()}>
              <span class="ml-auto text-[10px] text-zinc-500">
                filter: <span class="text-cyan-400">{selectedRole()}</span>
              </span>
            </Show>
          </div>

          <Show when={mainTab() === "activity"}>
            <div class="px-6 py-3 space-y-1.5">
              <Show
                when={activity().length > 0}
                fallback={
                  <div class="text-zinc-500 text-sm italic py-8 text-center">
                    {selectedRole()
                      ? `No recent activity for ${selectedRole()}. Click the role again to clear the filter.`
                      : "Factory is quiet. Type an intent above to start something."}
                  </div>
                }
              >
                <For each={activity()}>{(item) => (
                  <div class="flex items-baseline gap-3 text-sm py-1.5 border-b border-zinc-900/50 last:border-0">
                    <span class="text-zinc-500 tabular-nums w-12 shrink-0 text-right text-[11px]">
                      {ageSec(Math.max(0, Math.floor((Date.now() - item.ts) / 1000)))} ago
                    </span>
                    <span class={`${item.color} w-4 shrink-0 text-base`}>{item.icon}</span>
                    <div class="min-w-0 flex-1">
                      <div class="leading-relaxed">
                        <span class={`font-mono text-[12px] ${item.actorColor}`}>{item.actor}</span>
                        <span class="text-zinc-400 text-[13px]"> {item.verb} </span>
                        <span class="text-zinc-100 text-[13px]">{item.target}</span>
                      </div>
                      <Show when={item.detail}>
                        <div class="text-[11px] text-zinc-500 truncate font-mono mt-0.5">{item.detail}</div>
                      </Show>
                    </div>
                  </div>
                )}</For>
              </Show>
            </div>
          </Show>

          <Show when={mainTab() === "chat"}>
            <div class="px-6 py-3 space-y-3">
              <Show
                when={chatMessages().length > 0}
                fallback={
                  <div class="text-zinc-500 text-sm py-8 text-center">
                    <div class="italic mb-2">
                      {selectedRole()
                        ? `No recent messages for ${selectedRole()}.`
                        : "No chat traffic yet."}
                    </div>
                    <div class="text-[11px] text-zinc-600">
                      Type <code class="text-cyan-300">@cto draft an ADR for X</code> in the compose box to start a thread.
                    </div>
                  </div>
                }
              >
                <For each={chatMessages()}>{(msg) => {
                  const isOperator = msg.from_role === "operator";
                  return (
                    <div class="flex gap-3" classList={{ "justify-end": isOperator }}>
                      <div
                        class="max-w-2xl rounded-lg px-3 py-2 text-sm"
                        classList={{
                          "bg-cyan-900/40 border border-cyan-800": isOperator,
                          "bg-zinc-900 border border-zinc-700": !isOperator,
                        }}
                      >
                        <div class="flex items-baseline gap-2 mb-1">
                          <span class={`font-mono text-[11px] ${actorColorFor(msg.from_role)}`}>
                            {msg.from_role}
                          </span>
                          <Show when={msg.to_role && msg.to_role !== "all"}>
                            <span class="text-zinc-500 text-[10px]">→ {msg.to_role}</span>
                          </Show>
                          <span class="text-zinc-600 text-[10px] ml-auto">
                            {ageSinceIso(msg.created_at)} ago
                          </span>
                        </div>
                        <div class="text-zinc-100 whitespace-pre-wrap break-words leading-relaxed">
                          {msg.message}
                        </div>
                      </div>
                    </div>
                  );
                }}</For>
              </Show>
            </div>
          </Show>
        </main>
      </div>
    </div>
  );
};

function eventDecorate(evType: string): { icon: string; color: string; actor: string; verb: string; target: string } {
  if (evType === "twin_verdict") return { icon: "✓", color: "text-green-400", actor: "twin", verb: "decided on", target: "an action" };
  if (evType.startsWith("twin_")) return { icon: "✓", color: "text-green-400", actor: "twin", verb: "reviewed", target: evType.replace("twin_", "") };
  if (evType === "executor_applied" || evType === "file_write") return { icon: "✎", color: "text-cyan-300", actor: "executor", verb: "applied", target: "a file write" };
  if (evType === "persona_reply") return { icon: "💬", color: "text-purple-300", actor: "persona", verb: "replied", target: "to the board" };
  if (evType === "thought_journaled") return { icon: "✦", color: "text-purple-300", actor: "persona", verb: "journaled", target: "a thought" };
  if (evType.startsWith("improver_act")) return { icon: "▼", color: "text-cyan-400", actor: "improver", verb: "acted on", target: "a pattern" };
  if (evType.startsWith("improver_")) return { icon: "▼", color: "text-cyan-400", actor: "improver", verb: "ticked", target: "" };
  if (evType === "brain_tick") return { icon: "·", color: "text-zinc-500", actor: "brain", verb: "ticked", target: "" };
  if (evType.startsWith("commitment_created")) return { icon: "↑", color: "text-amber-300", actor: "commitment_parser", verb: "created", target: "a commitment" };
  if (evType.startsWith("commitment_satisfied")) return { icon: "✓", color: "text-green-300", actor: "executor", verb: "satisfied", target: "a commitment" };
  if (evType.startsWith("commitment_")) return { icon: "↑", color: "text-amber-300", actor: "commitment", verb: "updated", target: evType };
  if (evType.startsWith("escalat") || evType.startsWith("anomaly")) return { icon: "⚠", color: "text-red-400", actor: "system", verb: "escalated", target: "an issue" };
  if (evType === "loop_notification") return { icon: "🔔", color: "text-cyan-300", actor: "loop", verb: "notified", target: "" };
  return { icon: "·", color: "text-zinc-400", actor: "system", verb: "logged", target: evType };
}

/**
 * Translate raw live_event.preview payloads into one-line operator prose.
 * Falls back to a truncated raw string if we don't recognize the shape.
 */
function humanizePreview(evType: string, preview: string): string {
  if (!preview) return "";
  let parsed: any = null;
  try {
    parsed = JSON.parse(preview);
  } catch {
    return preview.length > 140 ? preview.slice(0, 140) + "…" : preview;
  }
  if (!parsed || typeof parsed !== "object") return String(preview).slice(0, 140);
  if (evType === "improver_tick" && parsed.by_source) {
    const keys = Object.keys(parsed.by_source);
    const total = keys.reduce((acc, k) => acc + (parsed.by_source[k] || 0), 0);
    return `${total} signal${total === 1 ? "" : "s"} across ${keys.length} pattern${keys.length === 1 ? "" : "s"}`;
  }
  if (evType === "twin_verdict") {
    const v = parsed.verdict || parsed.decision || "?";
    const aid = parsed.action_id ?? parsed.id ?? "";
    return `${v}${aid ? ` action #${aid}` : ""}`;
  }
  if (evType === "executor_applied" || evType === "file_write") {
    const path = parsed.path || parsed.target || "";
    const bytes = parsed.bytes ?? parsed.byte_len;
    return `${path}${bytes ? ` · ${bytes}B` : ""}`;
  }
  if (parsed.summary) return String(parsed.summary).slice(0, 140);
  if (parsed.message) return String(parsed.message).slice(0, 140);
  // Last resort: prettiest plausible field
  const firstScalar = Object.entries(parsed).find(([_, v]) => typeof v !== "object");
  if (firstScalar) return `${firstScalar[0]}: ${String(firstScalar[1]).slice(0, 100)}`;
  return "";
}

function actorColorFor(actor: string): string {
  if (!actor) return "text-zinc-400";
  if (actor === "operator") return "text-cyan-300";
  if (actor === "twin" || actor.endsWith("-twin")) return "text-green-300";
  if (actor === "executor" || actor.includes("executor")) return "text-cyan-300";
  if (actor === "improver" || actor === "brain") return "text-cyan-400";
  if (actor === "system" || actor === "loop") return "text-zinc-400";
  // c-suite roles → purple family; differentiate by stable hash for variety
  const palette = ["text-purple-300", "text-fuchsia-300", "text-pink-300", "text-indigo-300", "text-violet-300", "text-rose-300"];
  let h = 0;
  for (let i = 0; i < actor.length; i++) h = (h * 31 + actor.charCodeAt(i)) & 0xfffff;
  return palette[h % palette.length];
}

export default MissionControl;
