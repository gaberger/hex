/**
 * MissionControl.tsx — operator console, Hermes-shaped.
 *
 * ONE input. ONE stream. The c-suite is hidden machinery — operators
 * type to "the team", the runtime picks who replies. No factory rail
 * by default, no detail accordions, no filter chips, no pulse strip.
 *
 * Layout (full viewport, one column):
 *   [top bar]      status chips · escalations badge · [team ▸ toggle]
 *   [stream]       chat bubbles + autonomous-commit cards interleaved
 *   [compose]      one big input, sticky bottom
 *
 * Team panel slides in on demand for the rare case the operator wants
 * to address ONE persona specifically.
 */

import { Component, For, Index, Show, createSignal, onMount, onCleanup, createMemo, createEffect } from "solid-js";
import { restClient } from "../../services/rest-client";

interface PersonaRow { role: string; display_name: string; paused: boolean; last_tick_at: string; }
interface ExecutedRow { id: number; kind: string; path: string | null; success: boolean; error: string; executed_at: string; evidence: string; }
interface ActionRow { id: number; kind: string; proposed_by: string; status: string; twin_verdict: string; twin_rationale: string; escalate_reason: string; }
interface CommitmentRow { id: number; role: string; action: string; success_artifact: string; status: string; created_at: string; }
interface AttentionItem { id: string; priority: 0 | 1 | 2; kind: string; title: string; subtitle: string; age_seconds: number; cli_repro?: string; worktree_path?: string; branch?: string; }
interface ChatMessage { msg_id: number; from_role: string; to_role: string; message: string; created_at: string; }
interface Payload {
  stdb_alive: boolean;
  pulse?: { autonomous_commits_today?: number };
  personas: PersonaRow[];
  activity: { recent_executed: ExecutedRow[]; open_merge_requests: any[] };
  pending_decisions: { actions: ActionRow[]; commitments: CommitmentRow[]; anomalies: any[] };
  attention_feed?: AttentionItem[];
  recent_messages?: ChatMessage[];
}

const REFRESH_MS = 5000;
const PEER_CAPABILITIES: Record<string, string> = {
  ceo: "vision · prioritization",
  cto: "architecture · ADRs · security",
  cpo: "product · specs · UX",
  ciso: "security audits · OWASP",
  coo: "ops · runbooks · cost",
  "chief-visionary": "long-term strategy",
  "chief-architect": "system design",
  "product-lead": "feature shaping",
  "engineering-lead": "implementation",
  "design-lead": "UI / UX",
  "sre-lead": "incidents · SLOs",
  "validation-judge": "PASS / FAIL gates",
  "ux-designer": "WCAG audits (read-only)",
  "dashboard-ux-architect": "dashboard IA synthesis",
};
const ROLE_ORDER = ["ceo","cto","cpo","ciso","coo","chief-visionary","chief-architect","product-lead","engineering-lead","design-lead","sre-lead","validation-judge"];

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
const actorColor = (actor: string): string => {
  if (actor === "operator") return "text-cyan-300";
  if (actor.endsWith("-twin") || actor === "executor") return "text-green-300";
  const palette = ["text-purple-300","text-fuchsia-300","text-pink-300","text-indigo-300","text-violet-300","text-rose-300"];
  let h = 0;
  for (let i = 0; i < actor.length; i++) h = (h * 31 + actor.charCodeAt(i)) & 0xfffff;
  return palette[h % palette.length];
};

const MissionControl: Component = () => {
  const [data, setData] = createSignal<Payload | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [intent, setIntent] = createSignal("");
  const [running, setRunning] = createSignal(false);
  const [teamOpen, setTeamOpen] = createSignal(false);
  const [pendingChat, setPendingChat] = createSignal<{from: string; to: string; body: string; ts: number}[]>([]);
  const [attnBusy, setAttnBusy] = createSignal<string | null>(null);
  const [attnSuppressed, setAttnSuppressed] = createSignal<Set<string>>(new Set());

  let timer: ReturnType<typeof setInterval> | null = null;
  let streamScrollRef: HTMLElement | undefined;
  let lastStreamCount = 0;

  const refresh = async () => {
    try {
      const d = await restClient.get("/api/mission-control");
      setData(d);
      setError(null);
    } catch (e: any) {
      setError(e?.message || String(e));
    }
  };
  onMount(() => { refresh(); timer = setInterval(refresh, REFRESH_MS); });
  onCleanup(() => { if (timer) clearInterval(timer); });

  const suppressAttn = (id: string, cls?: string) => {
    const s = new Set(attnSuppressed());
    s.add(id);
    if (cls) s.add(cls);
    setAttnSuppressed(s);
    setTimeout(() => {
      const ns = new Set(attnSuppressed());
      ns.delete(id);
      if (cls) ns.delete(cls);
      setAttnSuppressed(ns);
    }, 5 * 60 * 1000);
  };

  // Send: leading @role addresses ONE persona; plain text broadcasts to
  // the c-suite (the team replies as a unit). Either way → one optimistic
  // bubble + one "thinking…" placeholder per recipient.
  const send = async () => {
    const text = intent().trim();
    if (!text || running()) return;
    setRunning(true);
    const now = Date.now();
    const m = text.match(/^@(\S+)\s+([\s\S]+)$/);
    const targetRole = m ? m[1] : null;
    const body = m ? m[2] : text;
    const route = m
      ? { from: "operator", to: targetRole, content: body }
      : { from: "operator", content: body };
    const recipients = m ? [targetRole!] : ["the team"];
    setPendingChat([
      ...pendingChat(),
      { from: "operator", to: recipients.join(", "), body, ts: now },
      ...recipients.map((r) => ({ from: r, to: "operator", body: "_thinking…_", ts: now + 1 })),
    ]);
    setIntent("");
    try {
      await restClient.post("/api/org/send-message", route);
      await refresh();
    } catch (e: any) {
      setError(`send: ${e?.message || e}`);
    } finally {
      setRunning(false);
      setTimeout(() => setPendingChat(pendingChat().filter((p) => Date.now() - p.ts < 60_000)), 60_000);
    }
  };

  const abandonAction = async (id: string, actionId: number) => {
    setAttnBusy(id);
    suppressAttn(id);
    try {
      await fetch(`/v1/database/hex/call/proposed_action_operator_override`, {
        method: "POST", headers: {"Content-Type":"application/json"},
        body: JSON.stringify([actionId, "rejected", "operator abandoned via dashboard"]),
      });
    } finally { setAttnBusy(null); await refresh(); }
  };
  const ackAnomaly = async (id: string, anomalyId: number, cls: string) => {
    setAttnBusy(id);
    suppressAttn(id, cls);
    try {
      await fetch(`/v1/database/hex/call/resource_anomaly_ack`, {
        method: "POST", headers: {"Content-Type":"application/json"},
        body: JSON.stringify([anomalyId, "operator-ack via dashboard"]),
      });
    } finally { setAttnBusy(null); await refresh(); }
  };
  const decideMerge = async (id: string, worktreePath: string, decision: "approved"|"rejected") => {
    setAttnBusy(id);
    suppressAttn(id);
    try {
      await fetch(`/v1/database/hex/call/merge_request_set_status`, {
        method: "POST", headers: {"Content-Type":"application/json"},
        body: JSON.stringify([worktreePath, decision]),
      });
    } finally { setAttnBusy(null); await refresh(); }
  };
  const restartStdb = async (id: string) => {
    if (!confirm("Restart SpacetimeDB? Loses in-memory state but reclaims RSS.")) return;
    setAttnBusy(id);
    try {
      await fetch(`/api/stdb/restart`, { method: "POST" });
      suppressAttn(id, "class:resource_anomaly:rss_oversize");
    } finally { setAttnBusy(null); setTimeout(refresh, 3000); }
  };

  // ── Unified stream: chat bubbles + autonomous commits + attention
  //    items, sorted oldest→newest so reads top-down like a real
  //    conversation. The c-suite is hidden machinery.
  interface StreamItem {
    kind: "chat" | "commit" | "attention";
    ts: number;
    chat?: { from: string; to: string; body: string; pending?: boolean };
    commit?: { id: number; path: string; actor: string };
    attention?: AttentionItem;
  }
  const stream = createMemo<StreamItem[]>(() => {
    const d = data();
    if (!d) return [];
    const items: StreamItem[] = [];

    // Real messages
    const realMsgs = [...(d.recent_messages || [])].sort((a, b) => (a.msg_id || 0) - (b.msg_id || 0));
    const realBodies = new Set(realMsgs.map((m) => `${m.from_role}:${m.message}`));
    const pendingFresh = pendingChat().filter((p) => {
      if (p.from === "operator" && realBodies.has(`operator:${p.body}`)) return false;
      if (p.body === "_thinking…_") {
        return !realMsgs.some((m) => m.from_role === p.from && tsToEpoch(m.created_at) >= p.ts - 1000);
      }
      return true;
    });
    // Collapse broadcast (same from+body within 5s) into one bubble
    const seen = new Map<string, string>();
    for (const msg of realMsgs) {
      const ts = tsToEpoch(msg.created_at) || msg.msg_id;
      const bucket = `${msg.from_role}|${Math.floor(ts / 5000)}|${msg.message}`;
      if (seen.has(bucket)) continue;
      seen.set(bucket, msg.from_role);
      items.push({ kind: "chat", ts, chat: { from: msg.from_role, to: msg.to_role || "everyone", body: msg.message } });
    }
    for (const p of pendingFresh) {
      items.push({ kind: "chat", ts: p.ts, chat: { from: p.from, to: p.to, body: p.body, pending: true } });
    }

    // Autonomous commits — inline cards in the stream
    for (const ex of d.activity?.recent_executed || []) {
      if (!ex.success || !ex.path) continue;
      const ts = tsToEpoch(ex.executed_at);
      if (!ts) continue;
      const m = ex.evidence?.match(/by (\S+):/);
      items.push({ kind: "commit", ts, commit: { id: ex.id, path: ex.path, actor: m ? m[1] : "executor" } });
    }

    // Attention items — inline cards, oldest at top
    const sup = attnSuppressed();
    const af = (d.attention_feed || []).filter((i) => {
      if (sup.has(i.id)) return false;
      const cls = `class:${i.kind}:${i.subtitle.slice(0, 40)}`;
      return !sup.has(cls);
    });
    for (const a of af) {
      const ts = Date.now() - a.age_seconds * 1000;
      items.push({ kind: "attention", ts, attention: a });
    }

    items.sort((a, b) => a.ts - b.ts);
    return items;
  });

  // Auto-scroll to bottom on new content (the most-recent is at bottom)
  createEffect(() => {
    const n = stream().length;
    if (n > lastStreamCount && streamScrollRef) {
      const el = streamScrollRef;
      const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 200;
      if (nearBottom || lastStreamCount === 0) {
        requestAnimationFrame(() => { el.scrollTop = el.scrollHeight; });
      }
    }
    lastStreamCount = n;
  });

  const attentionCount = () => (data()?.attention_feed || []).length;
  const personas = () => (data()?.personas || [])
    .slice()
    .sort((a, b) => {
      const ra = ROLE_ORDER.indexOf(a.role);
      const rb = ROLE_ORDER.indexOf(b.role);
      const ia = ra === -1 ? 99 : ra;
      const ib = rb === -1 ? 99 : rb;
      return ia - ib;
    });

  return (
    <div class="flex flex-col h-screen bg-zinc-950 text-zinc-100 font-sans">
      {/* ─── Minimal top bar ─── */}
      <header class="px-6 py-3 border-b border-zinc-800 flex items-center justify-between text-[11px]">
        <div class="flex items-baseline gap-3">
          <h1 class="text-base font-semibold tracking-tight">hex</h1>
        </div>
        <div class="flex items-center gap-2">
          <span class={data()?.stdb_alive ? "text-green-400" : "text-red-400"}>STDB {data()?.stdb_alive ? "✓" : "✗"}</span>
          <Show when={attentionCount() > 0}>
            <span class="text-zinc-500">·</span>
            <span class="text-amber-400" title={`${attentionCount()} attention items inline in the stream`}>
              {attentionCount()} attention
            </span>
          </Show>
          <span class="text-zinc-500">·</span>
          <button
            class="px-2 py-0.5 rounded border border-zinc-700 hover:bg-zinc-900 text-zinc-400"
            onClick={() => setTeamOpen(!teamOpen())}
            title="Toggle team list (12 personas). The team replies as a unit by default; use @role to address one."
          >
            team {teamOpen() ? "◂" : "▸"}
          </button>
        </div>
      </header>

      <Show when={error()}>
        <div class="px-6 py-2 bg-red-950/40 border-b border-red-900 text-red-300 text-xs">{error()}</div>
      </Show>

      <div class="flex-1 flex overflow-hidden">
        {/* ─── Optional team list (slides in from right when toggled) ─── */}
        <Show when={teamOpen()}>
          <aside class="w-64 shrink-0 border-l border-zinc-800 order-2 overflow-y-auto">
            <div class="px-4 py-3 border-b border-zinc-800 text-[10px] uppercase tracking-wide text-zinc-500">
              The team
            </div>
            <Index each={personas()}>{(pGet) => {
              const p = pGet();
              return (
                <button
                  class="w-full text-left px-4 py-2 border-b border-zinc-900 hover:bg-zinc-900"
                  onClick={() => { setIntent(`@${p.role} `); setTeamOpen(false); }}
                  title={`Address @${p.role} directly. ${PEER_CAPABILITIES[p.role] || ""}`}
                >
                  <div class="flex items-center gap-2 text-xs">
                    <span class={p.paused ? "text-yellow-400" : "text-green-400"}>●</span>
                    <span class="font-mono text-zinc-200">{p.role}</span>
                  </div>
                  <div class="text-[10px] text-zinc-500 mt-0.5">{PEER_CAPABILITIES[p.role] || "specialist"}</div>
                </button>
              );
            }}</Index>
          </aside>
        </Show>

        {/* ─── Main: one stream ─── */}
        <main ref={el => { streamScrollRef = el; }} class="flex-1 overflow-y-auto order-1">
          <div class="max-w-4xl mx-auto px-6 py-4 space-y-3">
            <Show when={stream().length === 0}>
              <div class="text-zinc-500 text-sm italic py-12 text-center">
                Quiet. Type below to start something.
              </div>
            </Show>
            <For each={stream()}>{(item) => (
              <Show when={item.kind === "chat" && item.chat}>{() => {
                const c = item.chat!;
                const isOp = c.from === "operator";
                return (
                  <div class="flex" classList={{ "justify-end": isOp }}>
                    <div
                      class="max-w-2xl rounded-lg px-3 py-2 text-sm"
                      classList={{
                        "bg-cyan-900/30 border border-cyan-800": isOp,
                        "bg-zinc-900 border border-zinc-700": !isOp,
                        "opacity-60 italic": !!c.pending,
                      }}
                    >
                      <div class="flex items-baseline gap-2 mb-1 text-[10px]">
                        <span class={`font-mono ${actorColor(c.from)}`}>{c.from}</span>
                        <Show when={c.to && c.to !== "operator"}>
                          <span class="text-zinc-600">→ {c.to}</span>
                        </Show>
                        <span class="text-zinc-600 ml-auto">
                          {ageSec(Math.max(0, Math.floor((Date.now() - item.ts) / 1000)))} ago
                        </span>
                      </div>
                      <div class="text-zinc-100 whitespace-pre-wrap break-words leading-relaxed">{c.body}</div>
                    </div>
                  </div>
                );
              }}</Show>
            )}</For>
            <For each={stream()}>{(item) => null /* solid quirk: re-renders */}</For>
            {/* Render commit + attention cards inline (separate For to keep DOM tidy) */}
            <For each={stream().filter((i) => i.kind !== "chat")}>{(item) => (
              <Show when={true}>
                {(() => {
                  if (item.kind === "commit" && item.commit) {
                    const c = item.commit;
                    const fname = c.path.split("/").pop() || c.path;
                    return (
                      <div class="flex justify-center">
                        <div class="text-[11px] text-zinc-500 px-3 py-1 rounded bg-cyan-950/20 border border-cyan-900/50">
                          <span class="text-cyan-400">✎</span> <span class={`font-mono ${actorColor(c.actor)}`}>{c.actor}</span>
                          {" "}wrote <span class="font-mono text-zinc-300">{fname}</span>{" "}
                          <span class="text-zinc-600">· {ageSec(Math.max(0, Math.floor((Date.now() - item.ts) / 1000)))} ago</span>
                        </div>
                      </div>
                    );
                  }
                  if (item.kind === "attention" && item.attention) {
                    const a = item.attention;
                    const numId = (() => {
                      const m = a.id.match(/^[a-z]+-(\d+)/);
                      return m ? parseInt(m[1], 10) : undefined;
                    })();
                    const isStdb = a.kind === "resource_anomaly" && (a.subtitle.includes("spacetimedb-standalone") || a.title.includes("rss_oversize"));
                    return (
                      <div class="flex justify-center">
                        <div
                          class="max-w-2xl w-full rounded-lg border px-3 py-2 text-sm"
                          classList={{
                            "border-red-800 bg-red-950/20": a.priority === 0,
                            "border-amber-800 bg-amber-950/10": a.priority === 1,
                            "border-zinc-800 bg-zinc-900/40": a.priority === 2,
                          }}
                        >
                          <div class="flex items-baseline gap-2 mb-1 text-[10px]">
                            <span class={a.priority === 0 ? "text-red-400" : a.priority === 1 ? "text-amber-400" : "text-blue-400"}>● {a.kind}</span>
                            <span class="text-zinc-600 ml-auto">{ageSec(a.age_seconds)}</span>
                          </div>
                          <div class="text-zinc-100">{a.title}</div>
                          <div class="text-[11px] text-zinc-500 mt-1">{a.subtitle}</div>
                          <div class="flex gap-1.5 mt-2">
                            <Show when={a.kind === "merge_vote_needed" && a.worktree_path}>
                              <button
                                class="px-2 py-0.5 rounded bg-green-900/40 hover:bg-green-900 border border-green-800 text-green-200 text-[10px] disabled:opacity-50"
                                disabled={attnBusy() === a.id}
                                onClick={() => decideMerge(a.id, a.worktree_path!, "approved")}
                              >Approve</button>
                              <button
                                class="px-2 py-0.5 rounded bg-red-900/40 hover:bg-red-900 border border-red-800 text-red-200 text-[10px] disabled:opacity-50"
                                disabled={attnBusy() === a.id}
                                onClick={() => decideMerge(a.id, a.worktree_path!, "rejected")}
                              >Reject</button>
                            </Show>
                            <Show when={isStdb && numId !== undefined}>
                              <button
                                class="px-2 py-0.5 rounded bg-amber-900/40 hover:bg-amber-900 border border-amber-800 text-amber-200 text-[10px] disabled:opacity-50"
                                disabled={attnBusy() === a.id}
                                onClick={() => restartStdb(a.id)}
                              >Restart STDB</button>
                            </Show>
                            <Show when={numId !== undefined && a.kind === "resource_anomaly"}>
                              <button
                                class="px-2 py-0.5 rounded bg-zinc-800 hover:bg-zinc-700 text-zinc-200 text-[10px] disabled:opacity-50"
                                disabled={attnBusy() === a.id}
                                onClick={() => ackAnomaly(a.id, numId!, `class:${a.kind}:${a.subtitle.slice(0, 40)}`)}
                              >Ack</button>
                            </Show>
                            <Show when={numId !== undefined && a.kind === "escalation"}>
                              <button
                                class="px-2 py-0.5 rounded bg-red-900/40 hover:bg-red-900 border border-red-800 text-red-200 text-[10px] disabled:opacity-50"
                                disabled={attnBusy() === a.id}
                                onClick={() => abandonAction(a.id, numId!)}
                              >Abandon</button>
                            </Show>
                          </div>
                        </div>
                      </div>
                    );
                  }
                  return null;
                })()}
              </Show>
            )}</For>
          </div>
        </main>
      </div>

      {/* ─── Compose (sticky bottom) ─── */}
      <div class="border-t border-zinc-800 bg-zinc-900/60 px-6 py-3">
        <div class="max-w-4xl mx-auto flex gap-2">
          <textarea
            class="flex-1 bg-zinc-950 border border-zinc-700 focus:border-cyan-600 focus:outline-none rounded px-3 py-2 text-sm font-mono resize-none"
            rows={2}
            placeholder='Tell the team. Plain text broadcasts to the c-suite. "@cto …" addresses one. ⌘↵ to send.'
            value={intent()}
            onInput={(e) => setIntent(e.currentTarget.value)}
            onKeyDown={(e) => { if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) { e.preventDefault(); send(); } }}
            disabled={running()}
          />
          <button
            class="px-5 rounded bg-cyan-700 hover:bg-cyan-600 text-white text-sm disabled:opacity-50"
            disabled={!intent().trim() || running()}
            onClick={send}
          >
            {running() ? "…" : "Send"}
          </button>
        </div>
      </div>
    </div>
  );
};

export default MissionControl;
