/**
 * MissionControl.tsx — single landing for the operator-debugger.
 *
 * User: the engineer building hex who needs to see what the factory is
 * doing right now and intervene when it goes off the rails.
 *
 * One workflow: dispatch · observe · intervene.
 *
 *   [header]   STDB · loops · auto-commits · TRIAGE button (when stuck) · KILL ALL
 *   [compose]  Dispatch agent run — natural language → /api/agent/run
 *   [factory]  Personas with capability tags (you know who to ask)
 *   [stream]   Unified activity feed (chat + commits + twin + anomalies),
 *              filter chips at top, click any row to inspect
 *
 * No tabs. No dual-mode compose. No drill-down footer.
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
  id: number; kind: string; path: string | null;
  success: boolean; error: string; executed_at: string; evidence: string;
}
interface ActionRow {
  id: number; kind: string; proposed_by: string; status: string;
  twin_verdict: string; twin_rationale: string; escalate_reason: string;
}
interface CommitmentRow {
  id: number; role: string; action: string; success_artifact: string;
  status: string; created_at: string;
}
interface AttentionItem {
  id: string; priority: 0 | 1 | 2; kind: string; title: string;
  subtitle: string; age_seconds: number; action_url?: string; cli_repro?: string;
}
interface LiveEvent {
  id: number; event_type: string; created_at: string; session_id: string; preview: string;
}
interface ChatMessage {
  msg_id: number; from_role: string; to_role: string;
  message: string; created_at: string;
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

// Role → capability — operator-readable description of what each persona is good at.
// When the operator wants to delegate, they pick from this catalog.
const ROLE_CAPABILITY: Record<string, string> = {
  ceo: "vision · prioritization · operator-broadcast",
  cto: "architecture · ADRs · security review",
  cpo: "product · specs · UX critique",
  ciso: "security audits · OWASP · threat model",
  coo: "ops · runbooks · cost discipline",
  "chief-visionary": "long-term strategy",
  "chief-architect": "system design · cross-cutting",
  "product-lead": "feature shaping · roadmap",
  "engineering-lead": "implementation · code review",
  "design-lead": "UI / UX",
  "sre-lead": "incidents · monitoring · SLOs",
  "validation-judge": "PASS / FAIL gates",
};

const MODELS = [
  { id: "anthropic/claude-haiku-4.5", label: "haiku" },
  { id: "anthropic/claude-sonnet-4-6", label: "sonnet" },
  { id: "anthropic/claude-opus-4-7", label: "opus" },
];

const ageSec = (s: number): string => {
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
};
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

type StreamFilter = "all" | "chat" | "commits" | "twin" | "anomaly";

const MissionControl: Component = () => {
  const [data, setData] = createSignal<Payload | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [intent, setIntent] = createSignal("");
  const [model, setModel] = createSignal(MODELS[0].id);
  const [maxIter, setMaxIter] = createSignal(6);
  const [running, setRunning] = createSignal(false);
  const [lastDispatch, setLastDispatch] = createSignal<string>("");
  const [streamFilter, setStreamFilter] = createSignal<StreamFilter>("all");
  const [killing, setKilling] = createSignal(false);
  const [askingRole, setAskingRole] = createSignal<string | null>(null);
  const [quickAsk, setQuickAsk] = createSignal("");

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

  const dispatch = async () => {
    const text = intent().trim();
    if (!text || running()) return;
    setRunning(true);
    setLastDispatch("dispatching…");
    try {
      const resp = await restClient.post("/api/agent/run", {
        intent: text,
        max_iterations: maxIter(),
        model: model(),
      });
      const steps = (resp?.steps || []).length;
      setLastDispatch(
        `✓ ${resp?.iterations ?? 0} iter · ${steps} step${steps === 1 ? "" : "s"} · ${resp?.stop_reason ?? "?"} · ${Math.round((resp?.elapsed_ms ?? 0) / 100) / 10}s`
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

  const askPersona = async (role: string) => {
    const text = quickAsk().trim();
    if (!text) return;
    try {
      await restClient.post("/api/org/send-message", {
        from: "operator", to: role, content: text,
      });
      setQuickAsk("");
      setAskingRole(null);
      setStreamFilter("chat");
      await refresh();
    } catch (e: any) {
      setError(`chat to ${role}: ${e?.message || e}`);
    }
  };

  const killAll = async (resume: boolean) => {
    if (killing()) return;
    if (!resume && !confirm("Pause every persona? They'll stop processing inbound work until resumed.")) return;
    setKilling(true);
    try {
      const roles = (data()?.personas || []).map((p) => p.role);
      for (const role of roles) {
        await fetch(`/v1/database/hex/call/persona_pool_set_paused`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify([role, !resume]),
        }).catch(() => null);
      }
      await refresh();
    } finally {
      setKilling(false);
    }
  };

  const allPaused = createMemo(() => {
    const ps = data()?.personas || [];
    return ps.length > 0 && ps.every((p) => p.paused);
  });

  // ── Factory rows with capability + open-work status ─────────────
  interface FactoryRow {
    role: string;
    paused: boolean;
    last_tick_at: string;
    capability: string;
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
        statusLine = `${esc} blocked`;
        statusColor = "text-red-400";
      } else if (open > 0) {
        statusLine = `${open} active`;
        statusColor = "text-cyan-300";
      } else {
        statusLine = `idle · ${age}`;
        statusColor = "text-zinc-500";
      }
      return {
        role: p.role,
        paused: p.paused,
        last_tick_at: p.last_tick_at,
        capability: ROLE_CAPABILITY[p.role] || "specialist",
        open, escalated: esc,
        statusLine, statusColor,
      };
    });
  });

  const attention = createMemo(() => data()?.attention_feed || []);
  const stuckEscalations = createMemo(() =>
    attention().filter((i) => i.priority === 0 && i.kind === "escalation").length
  );

  // ── Unified stream: commits + twin + chat + anomaly, filterable ─
  interface StreamItem {
    ts: number;
    kind: "commit" | "twin" | "chat" | "anomaly" | "other";
    icon: string;
    color: string;
    actor: string;
    actorColor: string;
    verb: string;
    target: string;
    detail?: string;
    body?: string;
    msgId?: number;
    sourceId: string | number;
  }
  const stream = createMemo<StreamItem[]>(() => {
    const d = data();
    if (!d) return [];
    const items: StreamItem[] = [];
    // autonomous commits
    for (const ex of d.activity?.recent_executed || []) {
      const ts = tsToEpoch(ex.executed_at);
      if (!ts) continue;
      const path = ex.path || "(unknown)";
      const filename = path.split("/").pop() || path;
      const m = ex.evidence?.match(/by (\S+):/);
      const actor = m ? m[1] : "executor";
      items.push({
        ts, kind: "commit",
        icon: ex.success ? "✎" : "✗",
        color: ex.success ? "text-cyan-300" : "text-red-400",
        actor, actorColor: actorColorFor(actor),
        verb: ex.success ? "wrote" : "tried to write",
        target: filename,
        detail: ex.success ? `${path} · action #${ex.id}` : (ex.error || `action #${ex.id}`),
        sourceId: ex.id,
      });
    }
    // chat — collapse broadcast (same from + body within 5s window) into
    // one bubble so the operator sees "operator → cto, cpo, ciso (3)"
    // rather than 7 identical copies of the same message routed to the
    // 7 c-suite roles.
    const chatMsgs = [...(d.recent_messages || [])].sort(
      (a, b) => (b.msg_id || 0) - (a.msg_id || 0)
    );
    const groups: Array<{ msgs: ChatMessage[]; ts: number; bucket: string }> = [];
    for (const msg of chatMsgs) {
      const ts = tsToEpoch(msg.created_at);
      const tsBucket = Math.floor(ts / 5000); // 5-second window
      const bucket = `${msg.from_role}|${tsBucket}|${msg.message}`;
      const existing = groups.find((g) => g.bucket === bucket);
      if (existing) {
        existing.msgs.push(msg);
      } else {
        groups.push({ msgs: [msg], ts: ts || (msg.msg_id || 0), bucket });
      }
    }
    for (const g of groups) {
      const first = g.msgs[0];
      const recipients = g.msgs.map((m) => m.to_role).filter((r): r is string => !!r && r !== "*");
      const targetLabel = recipients.length > 1
        ? `${recipients.slice(0, 3).join(", ")}${recipients.length > 3 ? ` + ${recipients.length - 3} more` : ""} (${recipients.length})`
        : (recipients[0] || "everyone");
      items.push({
        ts: g.ts,
        kind: "chat",
        icon: first.from_role === "operator" ? "→" : "💬",
        color: first.from_role === "operator" ? "text-cyan-300" : "text-purple-300",
        actor: first.from_role,
        actorColor: actorColorFor(first.from_role),
        verb: recipients.length > 1 ? "broadcast to" : "said to",
        target: targetLabel,
        body: first.message,
        msgId: first.msg_id,
        sourceId: `msg-${first.msg_id}`,
      });
    }
    // twin verdicts + anomalies + other live_events (skip heartbeats)
    for (const ev of d.live_events || []) {
      const ts = tsToEpoch(ev.created_at);
      if (!ts) continue;
      if (ev.event_type === "brain_tick" || ev.event_type === "improver_tick") continue;
      const info = eventDecorate(ev.event_type);
      let kind: StreamItem["kind"] = "other";
      if (ev.event_type.startsWith("twin_")) kind = "twin";
      else if (ev.event_type.startsWith("escalat") || ev.event_type.startsWith("anomaly")) kind = "anomaly";
      items.push({
        ts, kind,
        icon: info.icon, color: info.color,
        actor: info.actor, actorColor: actorColorFor(info.actor),
        verb: info.verb,
        target: info.target || ev.event_type,
        detail: ev.preview ? humanizePreview(ev.event_type, ev.preview) : undefined,
        sourceId: `ev-${ev.id}`,
      });
    }
    items.sort((a, b) => b.ts - a.ts);
    const f = streamFilter();
    const filtered = f === "all" ? items : items.filter((i) => i.kind === f);
    return filtered.slice(0, 80);
  });

  // Loop liveness
  const loopHealth = createMemo(() => {
    const evs = data()?.live_events || [];
    let brainTs = 0, improverTs = 0;
    for (const ev of evs) {
      const ts = tsToEpoch(ev.created_at);
      if (ev.event_type === "brain_tick" && ts > brainTs) brainTs = ts;
      if (ev.event_type === "improver_tick" && ts > improverTs) improverTs = ts;
    }
    return { brainTs, improverTs };
  });

  return (
    <div class="flex flex-col h-screen bg-zinc-950 text-zinc-100 font-sans">
      {/* ─── Header ─── */}
      <header class="px-6 py-3 border-b border-zinc-800 flex items-center justify-between flex-wrap gap-3">
        <div class="flex items-baseline gap-3">
          <h1 class="text-base font-semibold tracking-tight">hex</h1>
          <span class="text-[11px] text-zinc-500">operator console</span>
        </div>
        <div class="flex items-center gap-2 text-[11px]">
          <span class={data()?.stdb_alive ? "text-green-400" : "text-red-400"}>
            STDB {data()?.stdb_alive ? "✓" : "✗"}
          </span>
          <span class="text-zinc-500">·</span>
          <span
            class="text-zinc-500"
            title={`brain: ${loopHealth().brainTs ? ageSec(Math.max(0, Math.floor((Date.now() - loopHealth().brainTs) / 1000))) + " ago" : "silent"}\nimprover: ${loopHealth().improverTs ? ageSec(Math.max(0, Math.floor((Date.now() - loopHealth().improverTs) / 1000))) + " ago" : "silent"}`}
          >
            loops {loopHealth().brainTs && loopHealth().improverTs ? "✓" : "?"}
          </span>
          <span class="text-zinc-500">·</span>
          <span class="text-cyan-300 tabular-nums">
            {data()?.pulse?.autonomous_commits_today ?? "—"} commits today
          </span>
          <Show when={stuckEscalations() > 0}>
            <span class="text-zinc-500">·</span>
            <a
              href="#attention"
              class="px-2 py-1 rounded bg-red-900/40 hover:bg-red-900 border border-red-800 text-red-200 text-[11px]"
              title="P0 escalations awaiting triage"
            >
              {stuckEscalations()} escalations stuck — triage
            </a>
          </Show>
          <span class="text-zinc-500">·</span>
          <button
            class="px-2 py-1 rounded border text-[11px] disabled:opacity-50"
            classList={{
              "border-amber-700 bg-amber-900/30 text-amber-200 hover:bg-amber-900/60": !allPaused(),
              "border-green-700 bg-green-900/30 text-green-200 hover:bg-green-900/60": allPaused(),
            }}
            disabled={killing()}
            onClick={() => killAll(allPaused())}
            title={allPaused() ? "Resume all personas" : "Pause every persona — emergency stop"}
          >
            {killing() ? "…" : (allPaused() ? "▶ resume all" : "■ kill all")}
          </button>
        </div>
      </header>

      <Show when={error()}>
        <div class="px-6 py-2 bg-red-950/40 border-b border-red-900 text-red-300 text-xs">{error()}</div>
      </Show>

      {/* ─── Single-mode compose: dispatch agent run ─── */}
      <div class="px-6 py-3 border-b border-zinc-800 bg-zinc-900/40">
        <div class="flex items-center gap-2 mb-1.5 text-[11px] text-zinc-500">
          <span>Dispatch <span class="font-mono text-cyan-300">hex agent run</span> — natural-language intent → typed-tool loop</span>
          <span class="ml-auto flex items-center gap-2">
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
                type="number" min="1" max="20"
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
            placeholder='e.g. "Use code_patch to create docs/specs/foo.md with new_content ..."'
            value={intent()}
            onInput={(e) => setIntent(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); dispatch(); }
            }}
            disabled={running()}
          />
          <button
            class="px-5 rounded bg-cyan-700 hover:bg-cyan-600 text-white text-sm disabled:opacity-50"
            disabled={!intent().trim() || running()}
            onClick={dispatch}
          >
            {running() ? "Running…" : "Run"}
          </button>
        </div>
        <Show when={lastDispatch()}>
          <div class="text-[11px] text-zinc-400 mt-1.5 font-mono">{lastDispatch()}</div>
        </Show>
        <div class="text-[10px] text-zinc-600 mt-1">
          To chat with a persona instead: click "Ask" next to their row below.
        </div>
      </div>

      {/* ─── Two-column body ─── */}
      <div class="flex-1 grid grid-cols-12 gap-0 overflow-hidden">
        {/* Left: factory + attention */}
        <aside class="col-span-4 lg:col-span-3 border-r border-zinc-800 overflow-y-auto">
          <div class="px-4 py-3 border-b border-zinc-800">
            <h2 class="text-[10px] uppercase tracking-wide text-zinc-500 mb-2">Factory</h2>
            <div class="space-y-1.5">
              <For each={factoryRows()}>{(p) => (
                <div class="rounded border border-zinc-800 px-2 py-1.5">
                  <div class="flex items-center gap-2">
                    <span class={p.paused ? "text-yellow-400" : (p.escalated > 0 ? "text-red-400" : "text-green-400")}>●</span>
                    <span class="font-mono text-zinc-200 text-xs flex-1">{p.role}</span>
                    <span class={`text-[11px] ${p.statusColor}`}>{p.statusLine}</span>
                    <button
                      class="text-[10px] text-cyan-400 hover:text-cyan-300 hover:underline"
                      onClick={() => { setAskingRole(askingRole() === p.role ? null : p.role); setQuickAsk(""); }}
                    >
                      {askingRole() === p.role ? "cancel" : "ask"}
                    </button>
                  </div>
                  <div class="text-[10px] text-zinc-500 ml-4 mt-0.5">{p.capability}</div>
                  <Show when={askingRole() === p.role}>
                    <div class="mt-1.5 flex gap-1.5">
                      <input
                        class="flex-1 bg-zinc-950 border border-zinc-700 rounded px-2 py-1 text-[11px]"
                        placeholder={`message to @${p.role}…`}
                        value={quickAsk()}
                        onInput={(e) => setQuickAsk(e.currentTarget.value)}
                        onKeyDown={(e) => { if (e.key === "Enter") askPersona(p.role); }}
                        autofocus
                      />
                      <button
                        class="px-2 py-1 rounded bg-purple-700 hover:bg-purple-600 text-[10px] text-white disabled:opacity-50"
                        disabled={!quickAsk().trim()}
                        onClick={() => askPersona(p.role)}
                      >
                        Send
                      </button>
                    </div>
                  </Show>
                </div>
              )}</For>
            </div>
          </div>

          <div class="px-4 py-3" id="attention">
            <div class="flex items-center justify-between mb-2">
              <h2 class="text-[10px] uppercase tracking-wide text-zinc-500">Attention</h2>
              <span class="text-[10px] text-zinc-500 tabular-nums">{attention().length}</span>
            </div>
            <Show when={attention().length > 0} fallback={<div class="text-zinc-500 text-xs italic">Nothing waiting.</div>}>
              <div class="space-y-1">
                <For each={attention().slice(0, 25)}>{(item) => (
                  <div class="text-xs leading-tight rounded border px-2 py-1.5"
                    classList={{
                      "border-red-800 bg-red-950/20": item.priority === 0,
                      "border-amber-800 bg-amber-950/10": item.priority === 1,
                      "border-zinc-800": item.priority === 2,
                    }}
                  >
                    <div class="flex items-baseline gap-1.5">
                      <span class={item.priority === 0 ? "text-red-400" : item.priority === 1 ? "text-amber-400" : "text-blue-400"}>●</span>
                      <span class="text-zinc-200 truncate flex-1" title={item.title}>{item.title}</span>
                      <span class="text-zinc-500 tabular-nums shrink-0 text-[10px]">{ageSec(item.age_seconds)}</span>
                    </div>
                    <div class="text-[10px] text-zinc-500 mt-0.5 line-clamp-2">{item.subtitle}</div>
                  </div>
                )}</For>
              </div>
            </Show>
          </div>
        </aside>

        {/* Main: unified stream */}
        <main class="col-span-8 lg:col-span-9 overflow-y-auto flex flex-col">
          <div class="px-6 py-2 border-b border-zinc-800 sticky top-0 bg-zinc-950 z-10 flex items-center gap-1.5 flex-wrap">
            <For each={[
              { id: "all" as const, label: "All" },
              { id: "chat" as const, label: "Chat" },
              { id: "commits" as const, label: "Commits" },
              { id: "twin" as const, label: "Twin" },
              { id: "anomaly" as const, label: "Anomalies" },
            ]}>{(f) => (
              <button
                class="px-2 py-1 rounded text-[11px] border transition-colors"
                classList={{
                  "border-cyan-700 bg-cyan-900/30 text-cyan-200": streamFilter() === f.id,
                  "border-zinc-800 text-zinc-500 hover:bg-zinc-900 hover:text-zinc-300": streamFilter() !== f.id,
                }}
                onClick={() => setStreamFilter(f.id)}
              >
                {f.label}
              </button>
            )}</For>
            <span class="ml-auto text-[10px] text-zinc-500 tabular-nums">{stream().length} events</span>
          </div>

          <div class="px-6 py-3 space-y-2">
            <Show when={stream().length > 0} fallback={
              <div class="text-zinc-500 text-sm italic py-8 text-center">
                {streamFilter() === "all"
                  ? "Factory quiet. Dispatch a run above to start something."
                  : `No recent ${streamFilter()} events. Switch filter to "All" to see everything.`}
              </div>
            }>
              <For each={stream()}>{(item) => (
                <div class="border-b border-zinc-900/50 last:border-0 py-1.5">
                  <Show when={item.kind === "chat"} fallback={
                    <div class="flex items-baseline gap-3 text-sm">
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
                  }>
                    {/* Chat — bubble layout */}
                    <div class="flex gap-3" classList={{ "justify-end": item.actor === "operator" }}>
                      <div
                        class="max-w-3xl rounded-lg px-3 py-2 text-sm"
                        classList={{
                          "bg-cyan-900/30 border border-cyan-800": item.actor === "operator",
                          "bg-zinc-900 border border-zinc-700": item.actor !== "operator",
                        }}
                      >
                        <div class="flex items-baseline gap-2 mb-1 text-[10px]">
                          <span class={`font-mono ${item.actorColor}`}>{item.actor}</span>
                          <span class="text-zinc-600">→ {item.target}</span>
                          <span class="text-zinc-600 ml-auto">
                            {item.ts ? `${ageSec(Math.max(0, Math.floor((Date.now() - item.ts) / 1000)))} ago` : `#${item.msgId}`}
                          </span>
                        </div>
                        <div class="text-zinc-100 whitespace-pre-wrap break-words leading-relaxed">
                          {item.body || ""}
                        </div>
                      </div>
                    </div>
                  </Show>
                </div>
              )}</For>
            </Show>
          </div>
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
  if (evType.startsWith("commitment_created")) return { icon: "↑", color: "text-amber-300", actor: "commitment_parser", verb: "created", target: "a commitment" };
  if (evType.startsWith("commitment_satisfied")) return { icon: "✓", color: "text-green-300", actor: "executor", verb: "satisfied", target: "a commitment" };
  if (evType.startsWith("commitment_")) return { icon: "↑", color: "text-amber-300", actor: "commitment", verb: "updated", target: evType };
  if (evType.startsWith("escalat") || evType.startsWith("anomaly")) return { icon: "⚠", color: "text-red-400", actor: "system", verb: "escalated", target: "an issue" };
  if (evType === "loop_notification") return { icon: "🔔", color: "text-cyan-300", actor: "loop", verb: "notified", target: "" };
  return { icon: "·", color: "text-zinc-400", actor: "system", verb: "logged", target: evType };
}

function humanizePreview(evType: string, preview: string): string {
  if (!preview) return "";
  let parsed: any = null;
  try { parsed = JSON.parse(preview); } catch { return preview.length > 140 ? preview.slice(0, 140) + "…" : preview; }
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
  if (parsed.summary) return String(parsed.summary).slice(0, 140);
  if (parsed.message) return String(parsed.message).slice(0, 140);
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
  const palette = ["text-purple-300", "text-fuchsia-300", "text-pink-300", "text-indigo-300", "text-violet-300", "text-rose-300"];
  let h = 0;
  for (let i = 0; i < actor.length; i++) h = (h * 31 + actor.charCodeAt(i)) & 0xfffff;
  return palette[h % palette.length];
}

export default MissionControl;
