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

import { Component, For, Index, Show, createSignal, onMount, onCleanup, createMemo, createEffect } from "solid-js";
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
  worktree_path?: string; branch?: string;
}
interface LiveEvent {
  id: number; event_type: string; created_at: string; session_id: string; preview: string;
}
interface ChatMessage {
  msg_id: number; from_role: string; to_role: string;
  message: string; created_at: string;
}
interface ProcessRow {
  pid: number;
  argv: string;
  rss_kb: number;
  cpu_pct: number;
  state: string;
}
interface ThoughtRow {
  thought_id: number;
  agent_role: string;
  kind: string;
  content: string;
  related_msg_id: number;
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
  top_processes?: ProcessRow[];
  recent_thoughts?: ThoughtRow[];
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

// Stable canonical order for the factory list. STDB SQL returns rows in
// indeterminate order; without this the list would shuffle on every 5s
// refresh and the operator's "ask cto" click would land on whichever
// row happened to be in that slot. Executive tier first, then IC leads,
// then specialists. Unknown roles get appended in alphabetical order.
const ROLE_ORDER: string[] = [
  "ceo",
  "cto",
  "cpo",
  "ciso",
  "coo",
  "chief-visionary",
  "chief-architect",
  "product-lead",
  "engineering-lead",
  "design-lead",
  "sre-lead",
  "validation-judge",
];
const roleRank = (role: string): number => {
  const i = ROLE_ORDER.indexOf(role);
  return i === -1 ? ROLE_ORDER.length : i;
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

/**
 * Plain-English description of what each attention kind means and
 * what the operator is supposed to do with it. Rendered as a small
 * caption above each expanded attention card so the operator never
 * has to guess.
 */
function kindExplainer(kind: string): string {
  switch (kind) {
    case "escalation":
      return "A proposed action got stuck and auto-handling gave up. Click Abandon to permanently reject it.";
    case "overdue_commitment":
      return "A persona promised an artifact and missed its deadline. Abandon to clear the commitment, or wait for retry.";
    case "merge_vote_needed":
      return "A worktree opened a merge request. Approve (lands on main) or Reject (discards the worktree).";
    case "resource_anomaly":
      return "STDB or another process flagged unusual resource usage. Restart STDB to reclaim memory, or Ack to suppress for 5 minutes if you want to watch the trend.";
    case "agent_run_active":
      return "A `hex agent run` dispatch is in flight. No action required.";
    default:
      return "";
  }
}

type StreamFilter = "all" | "chat" | "commit" | "twin" | "anomaly";

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
  // Attention card interactivity
  const [attnOpen, setAttnOpen] = createSignal<Set<string>>(new Set());
  const [attnBusy, setAttnBusy] = createSignal<string | null>(null);
  const [attnStatus, setAttnStatus] = createSignal<Record<string, string>>({});
  const [copiedId, setCopiedId] = createSignal<string | null>(null);
  // Optimistic dismissal — supervisor re-emits same-(kind, pids) anomalies
  // every ~60s, so an Ack would visually "do nothing." We locally
  // suppress the item AND any near-duplicate signature for 5 minutes
  // after Ack/Abandon, giving the operator real persistent dismissal.
  // Keys: id, plus a signature "<kind>:<pids>" for ack-class dedup.
  const [attnSuppressed, setAttnSuppressed] = createSignal<Set<string>>(new Set());
  const suppressAttention = (id: string, signature?: string) => {
    const s = new Set(attnSuppressed());
    s.add(id);
    if (signature) s.add(signature);
    setAttnSuppressed(s);
    setTimeout(() => {
      const ns = new Set(attnSuppressed());
      ns.delete(id);
      if (signature) ns.delete(signature);
      setAttnSuppressed(ns);
    }, 5 * 60 * 1000);
  };
  // Optimistic chat bubbles — show "you said" + "<role> is thinking…"
  // immediately, replace with real STDB rows when they land.
  interface PendingChat { from: string; to: string; body: string; ts: number; }
  const [pendingChat, setPendingChat] = createSignal<PendingChat[]>([]);

  const toggleAttn = (id: string) => {
    // Default state is OPEN. attnOpen stores "closed:<id>" entries for
    // items the operator has explicitly collapsed.
    const key = `closed:${id}`;
    const s = new Set(attnOpen());
    if (s.has(key)) s.delete(key); else s.add(key);
    setAttnOpen(s);
  };
  const copyCli = (cli: string, id: string) => {
    try {
      navigator.clipboard?.writeText(cli);
      setCopiedId(id);
      setTimeout(() => { if (copiedId() === id) setCopiedId(null); }, 1500);
    } catch {}
  };
  const abandonAction = async (id: string, actionId: number) => {
    setAttnBusy(id);
    // Optimistic: hide immediately so operator sees the action take effect.
    suppressAttention(id);
    try {
      const r = await fetch(`/v1/database/hex/call/proposed_action_operator_override`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify([actionId, "rejected", "operator abandoned via dashboard 2026-05-15"]),
      });
      setAttnStatus({ ...attnStatus(), [id]: r.ok ? "✓ rejected" : `error: HTTP ${r.status}` });
    } catch (e: any) {
      setAttnStatus({ ...attnStatus(), [id]: `error: ${e?.message || e}` });
    } finally {
      setAttnBusy(null);
      await refresh();
    }
  };
  const restartStdb = async (id: string) => {
    if (!confirm("Restart SpacetimeDB? Loses in-memory state but reclaims RSS. Persistent storage is unaffected.")) return;
    setAttnBusy(id);
    setAttnStatus({ ...attnStatus(), [id]: "restarting STDB… (~5s)" });
    try {
      const r = await fetch(`/api/stdb/restart`, { method: "POST" });
      const body = await r.json().catch(() => ({}));
      if (r.ok && body.ok) {
        setAttnStatus({ ...attnStatus(), [id]: `✓ STDB restarted on port ${body.port}` });
        // Class-suppress same-anomaly so the rss_oversize doesn't immediately reappear
        suppressAttention(id, `class:resource_anomaly:rss_oversize`);
      } else {
        setAttnStatus({ ...attnStatus(), [id]: `error: ${body.error || `HTTP ${r.status}`}` });
      }
    } catch (e: any) {
      setAttnStatus({ ...attnStatus(), [id]: `error: ${e?.message || e}` });
    } finally {
      setAttnBusy(null);
      // Wait a beat for STDB to come back, then refresh
      setTimeout(() => refresh(), 3000);
    }
  };
  const decideMerge = async (id: string, worktreePath: string, decision: "approved" | "rejected") => {
    if (!worktreePath) return;
    setAttnBusy(id);
    suppressAttention(id);
    try {
      const r = await fetch(`/v1/database/hex/call/merge_request_set_status`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify([worktreePath, decision]),
      });
      setAttnStatus({
        ...attnStatus(),
        [id]: r.ok ? `✓ ${decision}` : `error: HTTP ${r.status}`,
      });
    } catch (e: any) {
      setAttnStatus({ ...attnStatus(), [id]: `error: ${e?.message || e}` });
    } finally {
      setAttnBusy(null);
      await refresh();
    }
  };
  const ackAnomaly = async (id: string, anomalyId: number) => {
    setAttnBusy(id);
    // Optimistic: hide this item AND any same-(kind, pids) re-emission.
    // The kind+pids signature is encoded in the item's pulled subtitle
    // (e.g. "PID 362979 RSS 25.0 GiB ..."); use the item itself + a
    // looser class signature derived from kind alone so RSS spikes
    // for OTHER pids still surface.
    const items = data()?.attention_feed || [];
    const me = items.find((i) => i.id === id);
    const sig = me ? `class:${me.kind}:${me.subtitle.slice(0, 40)}` : undefined;
    suppressAttention(id, sig);
    try {
      const r = await fetch(`/v1/database/hex/call/resource_anomaly_ack`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify([anomalyId, "operator-ack via dashboard 2026-05-15"]),
      });
      setAttnStatus({ ...attnStatus(), [id]: r.ok ? "✓ acked (suppressed for 5m)" : `error: HTTP ${r.status}` });
    } catch (e: any) {
      setAttnStatus({ ...attnStatus(), [id]: `error: ${e?.message || e}` });
    } finally {
      setAttnBusy(null);
      await refresh();
    }
  };

  let timer: ReturnType<typeof setInterval> | null = null;
  let mainScrollRef: HTMLElement | undefined;
  let lastChatCount = 0;
  let activeAskInput: HTMLInputElement | undefined;
  const refresh = async () => {
    // Pause data refresh while operator is typing in a per-persona
    // "ask" input — otherwise the 5s reconciliation can interrupt
    // focus, cursor position, or in-flight IME composition. Resumed
    // automatically once they Send or Cancel.
    if (askingRole() !== null) return;
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
  // Auto-scroll the main column to the bottom when chat filter is
  // active and new messages land. Only auto-scrolls if the operator is
  // already near the bottom — preserves their scroll position when
  // they're reading older messages.
  createEffect(() => {
    if (streamFilter() !== "chat") {
      lastChatCount = stream().length;
      return;
    }
    const newCount = stream().length;
    if (newCount > lastChatCount && mainScrollRef) {
      const el = mainScrollRef;
      const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 200;
      if (nearBottom || lastChatCount === 0) {
        // Defer to next frame so the new DOM is in place
        requestAnimationFrame(() => {
          el.scrollTop = el.scrollHeight;
        });
      }
    }
    lastChatCount = newCount;
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
    const now = Date.now();
    // Optimistic: render the operator's message + "thinking" bubble
    // immediately so the operator sees the send landed.
    setPendingChat([
      ...pendingChat(),
      { from: "operator", to: role, body: text, ts: now },
      { from: role, to: "operator", body: "_thinking…_", ts: now + 1 },
    ]);
    setQuickAsk("");
    setAskingRole(null);
    setStreamFilter("chat");
    try {
      await restClient.post("/api/org/send-message", {
        from: "operator", to: role, content: text,
      });
      await refresh();
    } catch (e: any) {
      setError(`chat to ${role}: ${e?.message || e}`);
    }
    // Clean up pending bubbles older than 60s — by then the real reply
    // either landed (showing as agent_messages) or the persona failed
    // to respond (operator can re-ask).
    setTimeout(() => {
      setPendingChat(pendingChat().filter((p) => Date.now() - p.ts < 60_000));
    }, 60_000);
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

  // ── Per-persona heat: how active is each role and what are they
  //    working on? Counts messages + thoughts + actions + commitments
  //    per role; the most recent content body becomes the "working on"
  //    subject. Used for the heatmap intensity + per-row subject line.
  interface PersonaHeat {
    count: number;
    workingOn: string;
    workingKind: string; // "thinking" | "said" | "committed" | "proposed"
  }
  const personaHeat = createMemo<Record<string, PersonaHeat>>(() => {
    const d = data();
    if (!d) return {};
    const heat: Record<string, PersonaHeat> = {};
    const touch = (role: string, body: string, kind: string) => {
      if (!role || role === "operator") return;
      const cur = heat[role];
      if (!cur) {
        heat[role] = { count: 1, workingOn: body, workingKind: kind };
      } else {
        cur.count += 1;
        // First touched wins as "latest" (sources are pre-sorted by
        // recency upstream: recent_thoughts and recent_messages are
        // most-recent-first).
      }
    };
    for (const t of d.recent_thoughts || []) {
      touch(t.agent_role, t.content || "", "thinking");
    }
    for (const m of d.recent_messages || []) {
      if (m.from_role === "operator") continue;
      touch(m.from_role, m.message || "", "said");
    }
    for (const c of d.pending_decisions?.commitments || []) {
      if (c.status === "satisfied") continue;
      touch(c.role, c.action || "", "committed");
    }
    for (const a of d.pending_decisions?.actions || []) {
      const by = a.proposed_by || "";
      if (by.includes(":") || by === "operator-passthrough") continue;
      touch(by, a.twin_rationale || a.escalate_reason || a.kind, "proposed");
    }
    return heat;
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
    heatCount: number;       // 0+ count of recent activity items
    heatTier: 0 | 1 | 2 | 3; // 0=cold, 1=warm, 2=hot, 3=very-hot
    workingOn: string;       // latest body snippet
    workingKind: string;     // thinking | said | committed | proposed
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
    const heat = personaHeat();
    const rows = (d.personas || []).map((p) => {
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
      const h = heat[p.role];
      const heatCount = h?.count || 0;
      // 0 cold · 1-2 warm · 3-4 hot · 5+ very hot
      const heatTier: 0 | 1 | 2 | 3 = heatCount === 0
        ? 0
        : heatCount <= 2 ? 1 : heatCount <= 4 ? 2 : 3;
      return {
        role: p.role,
        paused: p.paused,
        last_tick_at: p.last_tick_at,
        capability: ROLE_CAPABILITY[p.role] || "specialist",
        open, escalated: esc,
        statusLine, statusColor,
        heatCount,
        heatTier,
        workingOn: h?.workingOn || "",
        workingKind: h?.workingKind || "",
      };
    });
    // Stable canonical order — never shuffle on refresh. Unknown roles
    // sort alphabetically AFTER known canonical roles.
    rows.sort((a, b) => {
      const ra = roleRank(a.role);
      const rb = roleRank(b.role);
      if (ra !== rb) return ra - rb;
      return a.role.localeCompare(b.role);
    });
    return rows;
  });

  const attention = createMemo(() => {
    const items = data()?.attention_feed || [];
    const sup = attnSuppressed();
    if (sup.size === 0) return items;
    return items.filter((i) => {
      if (sup.has(i.id)) return false;
      // Class-signature suppression: same-kind near-duplicate within 5m
      const sig = `class:${i.kind}:${i.subtitle.slice(0, 40)}`;
      return !sup.has(sig);
    });
  });
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
    const realMsgs = [...(d.recent_messages || [])].sort(
      (a, b) => (b.msg_id || 0) - (a.msg_id || 0)
    );
    // Hide optimistic bubbles whose real STDB row has already landed.
    const realBodies = new Set(realMsgs.map((m) => `${m.from_role}:${m.message}`));
    const pendingFresh = pendingChat().filter((p) => {
      // Drop the operator's own optimistic bubble once the real row appears
      if (p.from === "operator" && realBodies.has(`operator:${p.body}`)) return false;
      // Drop the thinking placeholder once any real message from that persona appears since the pending ts
      if (p.body === "_thinking…_") {
        return !realMsgs.some(
          (m) => m.from_role === p.from && tsToEpoch(m.created_at) >= p.ts - 1000
        );
      }
      return true;
    });
    const pendingAsChat: ChatMessage[] = pendingFresh.map((p) => ({
      msg_id: -p.ts, // negative so they never collide with real ids
      from_role: p.from,
      to_role: p.to,
      message: p.body,
      created_at: new Date(p.ts).toISOString(),
    }));
    const chatMsgs = [...pendingAsChat, ...realMsgs];
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
    items.sort((a, b) => b.ts - a.ts); // newest-first (activity convention)
    const f = streamFilter();
    const filtered = f === "all" ? items : items.filter((i) => i.kind === f);
    const sliced = filtered.slice(0, 80);
    // Chat reads top-down (question → reply); reverse so most-recent
    // is at the bottom of the rendered list. Other filters keep the
    // newsfeed/log convention of newest at the top.
    return f === "chat" ? sliced.reverse() : sliced;
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
            <button
              class="px-2 py-1 rounded bg-red-900/40 hover:bg-red-900 border border-red-800 text-red-200 text-[11px]"
              title="P0 escalations awaiting triage"
              onClick={() => {
                document.getElementById("attention")?.scrollIntoView({ behavior: "smooth", block: "start" });
              }}
            >
              {stuckEscalations()} escalations stuck — triage
            </button>
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
              {/* Index instead of For — preserves DOM nodes across data
                  refreshes (every 5s), so the per-persona "ask" input
                  doesn't lose focus while the operator is typing.
                  Index keys by position; factoryRows is stable order
                  per ROLE_ORDER above. */}
              <Index each={factoryRows()}>{(pGet) => {
                const p = pGet;
                // Heat tier → left-border + background tint. The
                // colored edge gives an at-a-glance heatmap: cold
                // (zinc) → warm (cyan-900) → hot (amber-800) →
                // very hot (red-700).
                const heatBorderClass = (t: 0|1|2|3) => {
                  if (t === 3) return "border-l-red-600 bg-red-950/15";
                  if (t === 2) return "border-l-amber-600 bg-amber-950/10";
                  if (t === 1) return "border-l-cyan-700 bg-cyan-950/10";
                  return "border-l-zinc-700";
                };
                return (
                <div class={`rounded border border-zinc-800 border-l-4 px-2 py-1.5 ${heatBorderClass(p().heatTier)}`}>
                  <div class="flex items-center gap-2">
                    <span class={p().paused ? "text-yellow-400" : (p().escalated > 0 ? "text-red-400" : "text-green-400")}>●</span>
                    <span class="font-mono text-zinc-200 text-xs flex-1">{p().role}</span>
                    <Show when={p().heatCount > 0}>
                      <span class="text-[10px] text-zinc-400 tabular-nums" title={`${p().heatCount} recent activity items`}>
                        {p().heatCount}↑
                      </span>
                    </Show>
                    <span class={`text-[11px] ${p().statusColor}`}>{p().statusLine}</span>
                    <button
                      class="text-[10px] text-cyan-400 hover:text-cyan-300 hover:underline"
                      onClick={() => {
                        const wasAsking = askingRole() === p().role;
                        setAskingRole(wasAsking ? null : p().role);
                        setQuickAsk("");
                        // Cancel resumes auto-refresh — catch up now
                        if (wasAsking) refresh();
                      }}
                    >
                      {askingRole() === p().role ? "cancel" : "ask"}
                    </button>
                  </div>
                  <div class="text-[10px] text-zinc-500 ml-4 mt-0.5">{p().capability}</div>
                  <Show when={p().workingOn}>
                    <div class="text-[10px] ml-4 mt-0.5 line-clamp-2">
                      <span class="text-zinc-500">{p().workingKind}: </span>
                      <span class="text-zinc-300">{p().workingOn.slice(0, 120)}</span>
                    </div>
                  </Show>
                  <Show when={askingRole() === p().role}>
                    <div class="mt-1.5 flex gap-1.5">
                      <input
                        ref={(el) => {
                          activeAskInput = el;
                          // Solid's autofocus attribute is unreliable on
                          // hydrated nodes — do it imperatively after the
                          // element is in the DOM.
                          requestAnimationFrame(() => el.focus());
                        }}
                        class="flex-1 bg-zinc-950 border border-zinc-700 rounded px-2 py-1 text-[11px] focus:outline-none focus:border-purple-500"
                        placeholder={`message to @${p().role}… (auto-refresh paused)`}
                        value={quickAsk()}
                        onInput={(e) => setQuickAsk(e.currentTarget.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") askPersona(p().role);
                          if (e.key === "Escape") { setAskingRole(null); setQuickAsk(""); refresh(); }
                        }}
                      />
                      <button
                        class="px-2 py-1 rounded bg-purple-700 hover:bg-purple-600 text-[10px] text-white disabled:opacity-50"
                        disabled={!quickAsk().trim()}
                        onClick={() => askPersona(p().role)}
                      >
                        Send
                      </button>
                    </div>
                  </Show>
                </div>
              )}}</Index>
            </div>
          </div>

          <div class="px-4 py-3" id="attention">
            <div class="flex items-center justify-between mb-2">
              <h2 class="text-[10px] uppercase tracking-wide text-zinc-500">Attention</h2>
              <span class="text-[10px] text-zinc-500 tabular-nums">{attention().length}</span>
            </div>
            <Show when={attention().length > 0} fallback={<div class="text-zinc-500 text-xs italic">Nothing waiting.</div>}>
              <div class="space-y-1">
                <For each={attention().slice(0, 25)}>{(item) => {
                  // Default to OPEN so action buttons (Copy CLI, Show
                  // in stream, Abandon, Ack) are visible without
                  // requiring the operator to discover the click-to-
                  // expand affordance. attnOpen tracks explicitly
                  // CLOSED items by id.
                  const isOpen = () => !attnOpen().has(`closed:${item.id}`);
                  // Pull a numeric action id from id slugs like "escalation-12345"
                  const numId = (() => {
                    const m = item.id.match(/^[a-z]+-(\d+)/);
                    return m ? parseInt(m[1], 10) : undefined;
                  })();
                  return (
                    <div class="text-xs leading-tight rounded border"
                      classList={{
                        "border-red-800 bg-red-950/20": item.priority === 0,
                        "border-amber-800 bg-amber-950/10": item.priority === 1,
                        "border-zinc-800": item.priority === 2,
                      }}
                    >
                      <button
                        class="w-full text-left px-2 py-1.5 hover:bg-zinc-900/50 rounded-t"
                        onClick={() => toggleAttn(item.id)}
                      >
                        <div class="flex items-baseline gap-1.5">
                          <span class={item.priority === 0 ? "text-red-400" : item.priority === 1 ? "text-amber-400" : "text-blue-400"}>●</span>
                          <span class="text-zinc-200 truncate flex-1" title={item.title}>{item.title}</span>
                          <span class="text-zinc-500 tabular-nums shrink-0 text-[10px]">{ageSec(item.age_seconds)}</span>
                          <span class="text-zinc-600 shrink-0">{isOpen() ? "▾" : "▸"}</span>
                        </div>
                        <div class="text-[10px] text-zinc-500 mt-0.5 line-clamp-2">{item.subtitle}</div>
                      </button>
                      <Show when={isOpen()}>
                        <div class="border-t border-zinc-800 px-2 py-1.5 space-y-1.5 bg-zinc-950/60">
                          <div class="text-[10px] text-zinc-500 italic">
                            {kindExplainer(item.kind)}
                          </div>
                          <div class="flex flex-wrap gap-1.5">
                            <Show when={item.kind === "merge_vote_needed" && item.worktree_path}>
                              <button
                                class="px-2 py-0.5 rounded bg-green-900/40 hover:bg-green-900 border border-green-800 text-green-200 text-[10px] disabled:opacity-50"
                                disabled={attnBusy() === item.id}
                                onClick={() => decideMerge(item.id, item.worktree_path!, "approved")}
                              >
                                {attnBusy() === item.id ? "…" : "Approve merge"}
                              </button>
                              <button
                                class="px-2 py-0.5 rounded bg-red-900/40 hover:bg-red-900 border border-red-800 text-red-200 text-[10px] disabled:opacity-50"
                                disabled={attnBusy() === item.id}
                                onClick={() => decideMerge(item.id, item.worktree_path!, "rejected")}
                              >
                                {attnBusy() === item.id ? "…" : "Reject merge"}
                              </button>
                            </Show>
                            <Show when={numId !== undefined && item.kind === "escalation"}>
                              <button
                                class="px-2 py-0.5 rounded bg-red-900/40 hover:bg-red-900 border border-red-800 text-red-200 text-[10px] disabled:opacity-50"
                                disabled={attnBusy() === item.id}
                                onClick={() => abandonAction(item.id, numId!)}
                              >
                                {attnBusy() === item.id ? "…" : "Abandon"}
                              </button>
                            </Show>
                            <Show when={numId !== undefined && item.kind === "resource_anomaly"}>
                              <Show when={item.subtitle.includes("spacetimedb-standalone") || item.title.includes("rss_oversize")}>
                                <button
                                  class="px-2 py-0.5 rounded bg-amber-900/40 hover:bg-amber-900 border border-amber-800 text-amber-200 text-[10px] disabled:opacity-50"
                                  disabled={attnBusy() === item.id}
                                  onClick={() => restartStdb(item.id)}
                                >
                                  {attnBusy() === item.id ? "…" : "Restart STDB"}
                                </button>
                              </Show>
                              <button
                                class="px-2 py-0.5 rounded bg-zinc-800 hover:bg-zinc-700 text-zinc-200 text-[10px] disabled:opacity-50"
                                disabled={attnBusy() === item.id}
                                onClick={() => ackAnomaly(item.id, numId!)}
                              >
                                {attnBusy() === item.id ? "…" : "Ack"}
                              </button>
                            </Show>
                          </div>
                          <Show when={attnStatus()[item.id]}>
                            <div class="text-[10px] text-zinc-500 italic">{attnStatus()[item.id]}</div>
                          </Show>
                        </div>
                      </Show>
                    </div>
                  );
                }}</For>
              </div>
            </Show>
          </div>

          {/* Drill-down detail sections — formerly separate pages, now
              embedded inline so the operator never leaves Mission
              Control. Each section is a collapsible <details>. */}
          <details class="px-4 py-2 border-t border-zinc-800">
            <summary class="text-[10px] uppercase tracking-wide text-zinc-500 cursor-pointer hover:text-zinc-300">
              Top processes ({(data()?.top_processes || []).length})
            </summary>
            <div class="mt-2 space-y-1 text-[11px]">
              <For each={(data()?.top_processes || []).slice(0, 10)}>{(p) => (
                <div class="font-mono">
                  <div class="flex items-baseline gap-1.5">
                    <span class="text-cyan-400 tabular-nums shrink-0">{p.pid}</span>
                    <span class="text-zinc-200 tabular-nums shrink-0">{(p.rss_kb / 1024 / 1024).toFixed(1)}G</span>
                    <span class="text-zinc-500 tabular-nums shrink-0">{p.cpu_pct.toFixed(0)}%</span>
                    <span class="text-zinc-400 truncate">{p.argv}</span>
                  </div>
                </div>
              )}</For>
              <Show when={(data()?.top_processes || []).length === 0}>
                <div class="text-zinc-500 italic">No process data.</div>
              </Show>
            </div>
          </details>

          <details class="px-4 py-2 border-t border-zinc-800">
            <summary class="text-[10px] uppercase tracking-wide text-zinc-500 cursor-pointer hover:text-zinc-300">
              Commitments ({(data()?.pending_decisions?.commitments || []).length})
            </summary>
            <div class="mt-2 space-y-1.5 text-[11px]">
              <For each={(data()?.pending_decisions?.commitments || []).slice(0, 15)}>{(c) => (
                <div class="rounded border border-zinc-800 px-2 py-1">
                  <div class="flex items-baseline gap-1.5">
                    <span class="font-mono text-purple-300 shrink-0">{c.role}</span>
                    <span class="font-mono text-zinc-500 shrink-0">#{c.id}</span>
                    <span class="text-zinc-200 truncate flex-1" title={c.action}>{c.action}</span>
                    <span class="text-[10px] shrink-0"
                      classList={{
                        "text-amber-400": c.status === "open",
                        "text-red-400": c.status === "overdue" || c.status === "escalated",
                        "text-green-400": c.status === "satisfied",
                        "text-zinc-500": !["open","overdue","escalated","satisfied"].includes(c.status),
                      }}>
                      {c.status}
                    </span>
                  </div>
                  <Show when={c.success_artifact}>
                    <div class="text-[10px] text-zinc-500 mt-0.5 truncate">→ {c.success_artifact}</div>
                  </Show>
                </div>
              )}</For>
              <Show when={(data()?.pending_decisions?.commitments || []).length === 0}>
                <div class="text-zinc-500 italic">No open commitments.</div>
              </Show>
            </div>
          </details>

          <details class="px-4 py-2 border-t border-zinc-800">
            <summary class="text-[10px] uppercase tracking-wide text-zinc-500 cursor-pointer hover:text-zinc-300">
              Recent thoughts ({(data()?.recent_thoughts || []).length})
            </summary>
            <div class="mt-2 space-y-1 text-[11px]">
              <For each={(data()?.recent_thoughts || []).slice(0, 10)}>{(t) => (
                <div class="border-l-2 border-purple-800 pl-2">
                  <div class="flex items-baseline gap-1.5">
                    <span class="font-mono text-purple-300 shrink-0">{t.agent_role}</span>
                    <span class="text-zinc-500 shrink-0">{t.kind}</span>
                    <span class="text-zinc-600 shrink-0 ml-auto tabular-nums">{ageSinceAny(t.created_at)}</span>
                  </div>
                  <div class="text-zinc-300 mt-0.5 line-clamp-3">{t.content}</div>
                </div>
              )}</For>
              <Show when={(data()?.recent_thoughts || []).length === 0}>
                <div class="text-zinc-500 italic">No recent thoughts.</div>
              </Show>
            </div>
          </details>
        </aside>

        {/* Main: unified stream */}
        <main ref={el => { mainScrollRef = el; }} class="col-span-8 lg:col-span-9 overflow-y-auto flex flex-col">
          <div class="px-6 py-2 border-b border-zinc-800 sticky top-0 bg-zinc-950 z-10 flex items-center gap-1.5 flex-wrap">
            <For each={[
              { id: "all" as const, label: "All" },
              { id: "chat" as const, label: "Chat" },
              { id: "commit" as const, label: "Commits" },
              // Twin verdicts and anomalies don't flow through
              // live_events today — chip removed to avoid empty-state
              // confusion. Add back when the supervisor starts
              // publishing twin_verdict / anomaly events.
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
