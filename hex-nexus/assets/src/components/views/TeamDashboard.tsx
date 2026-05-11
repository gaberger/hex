/**
 * TeamDashboard.tsx — Interactive CEO dashboard with animated org communication
 *
 * Visual features:
 * - Live org chart with pulsing active agents
 * - Chat interface with @mention autocomplete
 * - Animated message routing visualization
 * - Real-time delegation chain display
 */

import { Component, For, Show, createSignal, onMount, onCleanup, createEffect, createMemo } from "solid-js";
import { restClient } from "../../services/rest-client";
import { orgCommsClient, type SendMessageResponse, type ConversationMessage, type DmMessage } from "../../services/org-comms";
import { projects } from "../../stores/projects";
import { hexfloConnected, swarmTasks, swarmAgents } from "../../stores/connection";

interface AgentOrgNode {
  name: string;
  role: string;
  tier: string;
  status?: string;
  last_heartbeat?: string | null;
  active_agents?: number;
  reports_to: string | null;
  direct_reports: string[];
  model?: {
    preferred?: string;
    fallback?: string;
    upgrade_threshold?: number;
  };
  context_level?: string;
}

// agent-comms reducer currently stores `""` for timestamp; fall back to a
// sentinel rather than rendering "Invalid Date" until the WASM module is
// republished with a real `ctx.timestamp` write.
function formatChatTime(ts: string | null | undefined): string {
  if (!ts) return "—";
  const d = new Date(ts);
  if (isNaN(d.getTime())) return "—";
  return d.toLocaleTimeString();
}

// SOP supervisor wraps every persona reply with a routing prefix like:
//   [cto] intent=code_question → spec_draft (rounds=3)  <real answer>
// or pure status stubs like:
//   [chief-visionary] Escalated: paradigm/strategy decision queued.
//   [cto] reasoning failed: openrouter HTTP 402 ...
// Mission Control rendered the raw content so the operator saw plumbing,
// not answers. Strip the prefix and classify status for a badge.
type ReplyStatus = "ok" | "escalated" | "failed";
interface ParsedReply {
  status: ReplyStatus;
  body: string;
  raw: string;
}
function parseReply(content: string | null | undefined): ParsedReply {
  const raw = content ?? "";
  // Match: [persona] ...→ ... (rounds=N) <body>
  const routed = raw.match(/^\[[\w-]+\][^\n]*?→[^\n]*?\(rounds=\d+\)\s*([\s\S]*)$/);
  if (routed) return { status: "ok", body: routed[1].trim() || "(empty reply)", raw };
  // Match: [persona] reasoning failed: ...
  const failed = raw.match(/^\[[\w-]+\]\s*reasoning failed:\s*([\s\S]*)$/i);
  if (failed) return { status: "failed", body: failed[1].trim(), raw };
  // Match: [persona] Escalated: ...
  const escalated = raw.match(/^\[[\w-]+\]\s*Escalated:\s*([\s\S]*)$/i);
  if (escalated) return { status: "escalated", body: escalated[1].trim(), raw };
  // Match: bare [persona] prefix with no routing
  const bare = raw.match(/^\[[\w-]+\]\s*([\s\S]*)$/);
  if (bare) return { status: "ok", body: bare[1].trim(), raw };
  return { status: "ok", body: raw, raw };
}

const ReplyBody: Component<{ content: string | null | undefined }> = (props) => {
  const parsed = () => parseReply(props.content);
  return (
    <div>
      <Show when={parsed().status !== "ok"}>
        <span class={`inline-block text-[10px] px-1.5 py-0.5 rounded mr-2 align-middle ${
          parsed().status === "failed"
            ? "bg-red-900 text-red-200"
            : "bg-yellow-900 text-yellow-200"
        }`}>
          {parsed().status === "failed" ? "✗ failed" : "⤴ escalated"}
        </span>
      </Show>
      <span class="whitespace-pre-wrap break-words">{parsed().body}</span>
    </div>
  );
};

interface MessageAnimation {
  id: string;
  from: string;
  to: string;
  progress: number; // 0-100
  status: "routing" | "processing" | "completed";
}

const TeamDashboard: Component = () => {
  const [nodes, setNodes] = createSignal<AgentOrgNode[]>([]);
  const [message, setMessage] = createSignal("");
  const [messages, setMessages] = createSignal<ConversationMessage[]>([]);
  const [activeAgents, setActiveAgents] = createSignal<string[]>([]);
  const [animations, setAnimations] = createSignal<MessageAnimation[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [showAutocomplete, setShowAutocomplete] = createSignal(false);
  const [autocompleteOptions, setAutocompleteOptions] = createSignal<string[]>([]);
  const [autocompleteType, setAutocompleteType] = createSignal<"agent" | "project">("agent");
  const [cursorPosition, setCursorPosition] = createSignal(0);

  // ── Debug drawer state ──────────────────────────────────────────────────
  const [showDebug, setShowDebug] = createSignal(true);
  const [lastSendResponse, setLastSendResponse] = createSignal<SendMessageResponse | null>(null);
  const [lastSendError, setLastSendError] = createSignal<string | null>(null);
  const [polledMessages, setPolledMessages] = createSignal<DmMessage[]>([]);
  const [pollStatus, setPollStatus] = createSignal<{ ok: boolean; at: string; msg: string }>({ ok: false, at: "—", msg: "idle" });
  const [debugLog, setDebugLog] = createSignal<{ ts: string; tag: string; text: string }[]>([]);
  const seenMessageIds = new Set<number>();

  // ── Chat persistence + "new chat" ─────────────────────────────────────────
  // chat_since_id is the server message id BELOW which polled messages should
  // be hidden from the thread. "New chat" sets it to the current max id.
  // Persisted in localStorage so reload preserves the conversation boundary.
  const SINCE_KEY = "hex.team.chatSinceId";
  const initialSince = (() => {
    try { return Number(localStorage.getItem(SINCE_KEY) || "0") || 0; }
    catch { return 0; }
  })();
  const [chatSinceId, setChatSinceIdRaw] = createSignal<number>(initialSince);
  const setChatSinceId = (n: number) => {
    setChatSinceIdRaw(n);
    try { localStorage.setItem(SINCE_KEY, String(n)); } catch {}
  };

  const log = (tag: string, text: string) => {
    const ts = new Date().toLocaleTimeString();
    setDebugLog(prev => [...prev.slice(-49), { ts, tag, text }]);
  };

  // Poll real DMs to ceo every 2s and merge into chat thread.
  let pollTimer: number | undefined;
  // First poll seeds history both directions (so reload restores the user's
  // own outbound msgs); subsequent polls only show inbound replies, since the
  // user's session-sent msgs are rendered optimistically by handleSend.
  let firstPoll = true;
  const pollMessages = async () => {
    try {
      const resp = await orgCommsClient.listMessages("ceo", 100);
      const msgs = resp.messages.slice().reverse(); // backend returns DESC; show oldest first
      setPolledMessages(msgs);
      setPollStatus({ ok: true, at: new Date().toLocaleTimeString(), msg: `${msgs.length} DM(s)` });

      // Merge new messages into the chat thread, deduped by server id.
      // First poll: seed BOTH directions to restore reload context.
      // Later polls: skip ceo→agent (optimistic local render handles those).
      let appended = 0;
      const since = chatSinceId();
      for (const m of msgs) {
        if (m.id == null || seenMessageIds.has(m.id)) continue;
        seenMessageIds.add(m.id);
        if (m.id <= since) continue; // hidden by "new chat" boundary
        if (!firstPoll && m.from === "ceo") continue;
        appended += 1;
        const ts = m.timestamp && !isNaN(new Date(m.timestamp).getTime())
          ? m.timestamp
          : new Date().toISOString();
        setMessages(prev => [...prev, {
          id: `dm-${m.id}`,
          from: m.from,
          to: m.to ?? "ceo",
          content: m.content,
          timestamp: ts,
          status: "completed",
        }]);
      }
      if (appended > 0) log(firstPoll ? "history" : "inbox", `+${appended} message(s) merged`);
      firstPoll = false;
    } catch (err: any) {
      setPollStatus({ ok: false, at: new Date().toLocaleTimeString(), msg: err?.message || String(err) });
      log("inbox-err", err?.message || String(err));
    }
  };

  onMount(async () => {
    try {
      // Use /api/hex-agents since /api/org/personas doesn't exist in current binary
      const response = await restClient.get("/api/hex-agents");
      const agents = response.agents || [];

      // Tier mapping based on role names
      const getTier = (role: string): string => {
        const executives = ["ceo", "cto", "coo", "cpo", "ciso", "chief-visionary"];
        const leads = ["engineering-lead", "product-lead", "sre-lead"];

        if (executives.includes(role)) return "executive";
        if (leads.includes(role)) return "lead";
        return "individual-contributor";
      };

      // Model info from persona YAMLs (hardcoded until we have API endpoint)
      const modelInfo: Record<string, { preferred: string; fallback: string; pricing: string }> = {
        "cto": { preferred: "opus-4-6", fallback: "sonnet-4-6", pricing: "$15/$75 per 1M tokens" },
        "coo": { preferred: "opus-4-6", fallback: "sonnet-4-6", pricing: "$15/$75 per 1M tokens" },
        "cpo": { preferred: "opus-4-6", fallback: "sonnet-4-6", pricing: "$15/$75 per 1M tokens" },
        "ciso": { preferred: "opus-4-6", fallback: "sonnet-4-6", pricing: "$15/$75 per 1M tokens" },
        "chief-visionary": { preferred: "opus-4-6", fallback: "sonnet-4-6", pricing: "$15/$75 per 1M tokens" },
        "engineering-lead": { preferred: "sonnet-4-6", fallback: "haiku-4-5", pricing: "$3/$15 per 1M tokens" },
        "product-lead": { preferred: "sonnet-4-6", fallback: "haiku-4-5", pricing: "$3/$15 per 1M tokens" },
        "sre-lead": { preferred: "sonnet-4-6", fallback: "haiku-4-5", pricing: "$3/$15 per 1M tokens" },
      };

      // Transform to AgentOrgNode format, filter online only, deduplicate by role
      const onlineAgents = agents.filter((a: any) => a.status === "online");

      // Deduplicate by role - keep the most recent registration
      const uniqueByRole = new Map<string, any>();
      onlineAgents.forEach((a: any) => {
        const role = a.role || a.name || "unknown";
        if (role === "ceo") return; // Skip CEO - user is the CEO

        const existing = uniqueByRole.get(role);
        if (!existing || a.registered_at > existing.registered_at) {
          uniqueByRole.set(role, a);
        }
      });

      const transformed = Array.from(uniqueByRole.values()).map((a: any) => {
        const role = a.role || a.name || "unknown";
        const tier = getTier(role);
        const modelData = modelInfo[role] || { preferred: "default", fallback: "free", pricing: "free" };

        return {
          name: role,
          role: role,
          tier: tier,
          status: a.status || "offline",
          last_heartbeat: a.last_heartbeat,
          reports_to: null,
          direct_reports: [],
          model: {
            preferred: modelData.preferred,
            fallback: modelData.fallback,
            upgrade_threshold: 0.8,
          },
          context_level: tier === "executive" ? "L3" : "L2",
          pricing: modelData.pricing,
        };
      });

      setNodes(transformed);
      setLoading(false);

      // Mark agents with status "online" as active
      const active = transformed
        .filter((p: AgentOrgNode) => p.status === "online")
        .map((p: AgentOrgNode) => p.name);
      setActiveAgents(active);

      // Start inbox polling for real DMs to ceo.
      log("init", `polling /api/org/messages?agent=ceo every 2s`);
      pollMessages();
      pollTimer = window.setInterval(pollMessages, 2000);
    } catch (err) {
      console.error("Failed to load org chart:", err);
      setLoading(false);
    }
  });

  onCleanup(() => {
    if (pollTimer !== undefined) window.clearInterval(pollTimer);
  });

  // Extract @mentions from message
  const mentions = createMemo(() => {
    const text = message();
    const matches = text.match(/@([a-z-]+)/g);
    return matches ? matches.map(m => m.substring(1)) : [];
  });

  // Handle input changes and trigger autocomplete
  const handleInput = (e: InputEvent) => {
    const input = e.currentTarget as HTMLInputElement;
    const text = input.value;
    const cursor = input.selectionStart || 0;

    setMessage(text);
    setCursorPosition(cursor);

    // Find if we're typing @ or #
    const beforeCursor = text.substring(0, cursor);
    const lastAt = beforeCursor.lastIndexOf("@");
    const lastHash = beforeCursor.lastIndexOf("#");

    const triggerPos = Math.max(lastAt, lastHash);

    if (triggerPos >= 0 && triggerPos === lastAt) {
      // @ autocomplete for agents
      const afterTrigger = beforeCursor.substring(lastAt + 1);
      const spaceAfter = afterTrigger.indexOf(" ");

      if (spaceAfter === -1 || spaceAfter === afterTrigger.length - 1) {
        const query = afterTrigger.toLowerCase();
        const filtered = nodes()
          .map(n => n.name)
          .filter(name => name.toLowerCase().includes(query))
          .slice(0, 5);

        if (filtered.length > 0) {
          setAutocompleteType("agent");
          setAutocompleteOptions(filtered);
          setShowAutocomplete(true);
          return;
        }
      }
    } else if (triggerPos >= 0 && triggerPos === lastHash) {
      // # autocomplete for projects
      const afterTrigger = beforeCursor.substring(lastHash + 1);
      const spaceAfter = afterTrigger.indexOf(" ");

      if (spaceAfter === -1 || spaceAfter === afterTrigger.length - 1) {
        const query = afterTrigger.toLowerCase();
        const allProjects = projects();

        console.log("Project autocomplete triggered:", { query, projectCount: allProjects.length });

        const filtered = allProjects
          .map(p => p.name || p.id)  // Use name if available, fallback to id
          .filter(name => name.toLowerCase().includes(query))
          .slice(0, 5);

        console.log("Filtered projects:", filtered);

        if (filtered.length > 0) {
          setAutocompleteType("project");
          setAutocompleteOptions(filtered);
          setShowAutocomplete(true);
          return;
        }
      }
    }

    setShowAutocomplete(false);
  };

  const selectAutocomplete = (option: string) => {
    const text = message();
    const cursor = cursorPosition();
    const beforeCursor = text.substring(0, cursor);

    const trigger = autocompleteType() === "agent" ? "@" : "#";
    const lastTrigger = beforeCursor.lastIndexOf(trigger);

    if (lastTrigger >= 0) {
      const before = text.substring(0, lastTrigger + 1);
      const after = text.substring(cursor);
      setMessage(before + option + " " + after);
    }

    setShowAutocomplete(false);
  };

  const handleSend = async () => {
    const text = message();
    if (!text.trim()) return;

    const mentionList = mentions();

    // Add user message to thread
    const userMsg: ConversationMessage = {
      id: `msg-${Date.now()}-${Math.random().toString(36).substring(2, 9)}`,
      from: "ceo",
      to: mentionList.length > 0 ? mentionList[0] : "(all-execs)",
      content: text,
      timestamp: new Date().toISOString(),
      status: "sent",
    };
    setMessages(prev => [...prev, userMsg]);
    setMessage("");

    log("send", `POST /api/org/send-message — mentions=[${mentionList.join(",") || "none → board meeting"}]`);

    try {
      const response = await orgCommsClient.sendMessage({
        from: "ceo",
        content: text,
      });
      setLastSendResponse(response);
      setLastSendError(null);
      log("ok", `routed_to=[${response.routed_to.join(",")}] msg_id=${response.message_id}`);

      // Routing animation only (no fake reply — real replies come via inbox poll).
      response.routed_to.forEach((agent, idx) => {
        setTimeout(() => {
          setActiveAgents(prev => [...prev, agent]);
          const anim: MessageAnimation = {
            id: `anim-${Date.now()}-${Math.random().toString(36).substring(2, 9)}`,
            from: "ceo",
            to: agent,
            progress: 0,
            status: "routing",
          };
          setAnimations(prev => [...prev, anim]);

          let progress = 0;
          const interval = setInterval(() => {
            progress += 5;
            setAnimations(prev => prev.map(a => a.id === anim.id ? { ...a, progress } : a));
            if (progress >= 100) {
              clearInterval(interval);
              setAnimations(prev => prev.map(a => a.id === anim.id ? { ...a, status: "completed" } : a));
              setActiveAgents(prev => prev.filter(a => a !== agent));
              setTimeout(() => {
                setAnimations(prev => prev.filter(a => a.id !== anim.id));
              }, 2000);
            }
          }, 30);
        }, idx * 200);
      });
    } catch (err: any) {
      const msg = err?.message || String(err);
      setLastSendError(msg);
      log("send-err", msg);
      console.error("Failed to send message:", err);
    }
  };

  const isAgentActive = (name: string) => activeAgents().includes(name);

  // Get agent status from REST API + SpacetimeDB
  const getAgentStatus = (name: string) => {
    const node = nodes().find(n => n.name === name);
    const tasks = swarmTasks().filter((t: any) =>
      t.assignedAgent === name && t.status === "in_progress"
    );

    // Use status from REST API if available, otherwise fallback to SpacetimeDB check
    const apiStatus = node?.status || "offline";
    const isOnline = apiStatus === "online";
    const agentCount = node?.active_agents || 0;

    return {
      isActive: tasks.length > 0,
      taskCount: tasks.length,
      lastSeen: node?.last_heartbeat || null,
      status: tasks.length > 0 ? "busy" : isOnline ? "idle" : "offline",
      agentCount
    };
  };

  const tierColor = (tier: string) => {
    switch (tier) {
      case "executive": return "bg-purple-600";
      case "lead": return "bg-blue-600";
      case "ic": return "bg-green-600";
      default: return "bg-gray-600";
    }
  };

  const statusColor = (status: string) => {
    switch (status) {
      case "busy": return "text-yellow-400";
      case "idle": return "text-green-400";
      case "offline": return "text-gray-500";
      default: return "text-gray-500";
    }
  };

  // Model metadata lookup
  const getModelInfo = (modelName?: string) => {
    const name = modelName || "unknown";

    // Cost per 1M tokens (input/output average)
    const costs: Record<string, { cost: number; context: string }> = {
      "claude-opus-4-7": { cost: 18, context: "200K" },
      "claude-sonnet-4-6": { cost: 4.5, context: "200K" },
      "claude-sonnet-4-5": { cost: 4.5, context: "200K" },
      "claude-haiku-4-5": { cost: 1.25, context: "200K" },
      "gpt-4": { cost: 45, context: "128K" },
      "gpt-4-turbo": { cost: 15, context: "128K" },
      "gpt-3.5-turbo": { cost: 1.5, context: "16K" },
      "qwen3:4b": { cost: 0, context: "32K" },
      "qwen2.5-coder:32b": { cost: 0, context: "128K" },
      "devstral-small-2:24b": { cost: 0, context: "128K" },
      "gemma4:latest": { cost: 0, context: "8K" },
    };

    return costs[name] || { cost: 0, context: "?" };
  };

  const AgentCard: Component<{ agent: AgentOrgNode }> = (props) => {
    const isActive = () => isAgentActive(props.agent.name);
    const status = createMemo(() => getAgentStatus(props.agent.name));

    return (
      <div
        class={`
          relative p-3 rounded-lg border-2 transition-all duration-300
          ${status().status === "online" || status().status === "idle"
            ? `${tierColor(props.agent.tier)} border-green-500/50`
            : "bg-gray-800 border-gray-700"
          }
        `}
      >
        {/* Status indicator dot */}
        <div class={`absolute top-2 right-2 w-2 h-2 rounded-full ${
          status().status === "busy" ? "bg-yellow-400" :
          status().status === "idle" ? "bg-green-400" :
          "bg-gray-600"
        }`}></div>

        <div class="text-white font-semibold text-sm">{props.agent.name}</div>
        <div class="text-white/80 text-xs mt-1">{props.agent.role}</div>

        <div class="mt-2 space-y-1">
          <div class="flex items-center justify-between">
            <div class="text-xs px-2 py-0.5 rounded bg-black/20 text-white/90">
              {props.agent.tier}
            </div>

            <div class="text-xs font-semibold text-white/90">
              {status().status}
              <Show when={status().agentCount > 0}>
                <span class="ml-1">({status().agentCount})</span>
              </Show>
            </div>
          </div>

          {/* Model info - hardcoded until backend YAMLs load correctly */}
          {(() => {
            const modelMap: Record<string, string> = {
              "ceo": "claude-opus-4-7",
              "cto": "claude-opus-4-7",
              "cpo": "claude-opus-4-7",
              "coo": "claude-opus-4-7",
              "ciso": "claude-opus-4-7",
              "chief-visionary": "claude-opus-4-7",
              "engineering-lead": "claude-sonnet-4-6",
              "product-lead": "claude-sonnet-4-6",
              "sre-lead": "claude-sonnet-4-6",
              "hex-coder": "qwen2.5-coder:32b",
              "hex-tester": "qwen2.5-coder:32b",
              "hex-fixer": "qwen2.5-coder:32b",
            };
            const modelName = props.agent.model?.preferred || modelMap[props.agent.name] || "qwen2.5-coder:32b";
            const info = getModelInfo(modelName);
            // Format model name for display
            const displayName = modelName
              .replace('claude-', '')
              .replace('qwen2.5-coder:', 'qwen2.5:')
              .replace(':32b', '-32b');
            return (
              <div class="text-xs text-white/70 flex items-center justify-between gap-2">
                <span class="truncate flex-1" title={modelName}>
                  {displayName}
                </span>
                <span>{info.context}</span>
                <Show when={info.cost > 0}>
                  <span>${info.cost}/M</span>
                </Show>
                <Show when={info.cost === 0}>
                  <span>free</span>
                </Show>
              </div>
            );
          })()}
        </div>
      </div>
    );
  };

  return (
    <div class="flex h-screen bg-gray-950">
      {/* Left: Org Chart with live status */}
      <div class="w-1/3 border-r border-gray-800 p-6 overflow-y-auto">
        <div class="flex items-center justify-between mb-4">
          <h2 class="text-xl font-bold text-white">Your Team</h2>
          <div class={`flex items-center gap-2 text-xs ${hexfloConnected() ? "text-green-400" : "text-yellow-400"}`}>
            <div class={`w-2 h-2 rounded-full ${hexfloConnected() ? "bg-green-400" : "bg-yellow-400 animate-pulse"}`}></div>
            {hexfloConnected() ? "Connected" : "Connecting..."}
          </div>
        </div>
        <p class="text-gray-400 text-sm mb-6">Live status • {activeAgents().length} active</p>

        <Show when={loading()}>
          <div class="text-gray-400">Loading org chart...</div>
        </Show>

        <Show when={!loading()}>
          {/* Group by tier */}
          <div class="space-y-6">
            <div>
              <div class="text-xs text-gray-500 uppercase tracking-wider mb-2">Executives</div>
              <div class="grid grid-cols-2 gap-2">
                <For each={nodes().filter(n => n.tier === "executive")}>
                  {(agent) => <AgentCard agent={agent} />}
                </For>
              </div>
            </div>

            <div>
              <div class="text-xs text-gray-500 uppercase tracking-wider mb-2">Leads</div>
              <div class="grid grid-cols-2 gap-2">
                <For each={nodes().filter(n => n.tier === "lead")}>
                  {(agent) => <AgentCard agent={agent} />}
                </For>
              </div>
            </div>

            <div>
              <div class="text-xs text-gray-500 uppercase tracking-wider mb-2">Individual Contributors ({nodes().filter(n => n.tier === "individual-contributor").length})</div>
              <div class="grid grid-cols-2 gap-2">
                <For each={nodes().filter(n => n.tier === "individual-contributor")}>
                  {(agent) => <AgentCard agent={agent} />}
                </For>
              </div>
            </div>
          </div>
        </Show>
      </div>

      {/* Center: Chat Interface */}
      <div class="flex-1 flex flex-col">
        {/* Header */}
        <div class="p-6 border-b border-gray-800">
          <div class="flex items-start justify-between gap-4">
            <div>
              <h1 class="text-2xl font-bold text-white">CEO Command Center</h1>
              <p class="text-gray-400 mt-1">
                Use @mentions to route messages: <span class="text-cyan-400">@cto</span>, <span class="text-purple-400">@cpo</span>, <span class="text-blue-400">@coo</span>.
                No @mentions → all executives (board meeting). Use <span class="text-green-400">#project-name</span> to scope.
              </p>
            </div>
            <button
              class="shrink-0 px-3 py-1.5 rounded bg-gray-800 hover:bg-gray-700 text-gray-200 text-sm border border-gray-700"
              title="Hide all prior messages from this thread. Past messages remain in STDB and the boundary persists across reloads."
              onClick={() => {
                const maxId = polledMessages().reduce((acc, m) => (m.id != null && m.id > acc ? m.id : acc), 0);
                setChatSinceId(maxId);
                setMessages([]);
                log("new-chat", `boundary moved to msg_id=${maxId}`);
              }}
            >
              New chat
            </button>
          </div>
        </div>

        {/* Messages */}
        <div class="flex-1 overflow-y-auto p-6 space-y-4">
          <For each={messages()}>
            {(msg) => (
              <div class={`flex ${msg.from === "ceo" ? "justify-end" : "justify-start"}`}>
                <div class={`
                  max-w-[70%] p-4 rounded-lg
                  ${msg.from === "ceo"
                    ? "bg-cyan-900 text-white"
                    : "bg-gray-800 text-gray-100"
                  }
                `}>
                  <div class="flex items-center gap-2 mb-1">
                    <div class="font-semibold text-sm">
                      {msg.from === "ceo" ? "You (CEO)" : msg.from}
                    </div>
                    <div class="text-xs text-gray-400">
                      {formatChatTime(msg.timestamp)}
                    </div>
                  </div>
                  <div class="text-sm"><ReplyBody content={msg.content} /></div>
                </div>
              </div>
            )}
          </For>
        </div>

        {/* Input */}
        <div class="p-6 border-t border-gray-800">
          <div class="relative">
            <div class="flex gap-3">
              <input
                type="text"
                value={message()}
                onInput={handleInput}
                onKeyPress={(e) => e.key === "Enter" && !showAutocomplete() && handleSend()}
                placeholder="Message your team... (use @cto, #project, etc.)"
                class="flex-1 bg-gray-800 text-white px-4 py-3 rounded-lg border border-gray-700 focus:border-cyan-500 focus:outline-none"
              />
              <button
                onClick={handleSend}
                disabled={!message().trim()}
                class="px-6 py-3 bg-cyan-600 hover:bg-cyan-500 disabled:bg-gray-700 disabled:text-gray-500 text-white font-semibold rounded-lg transition-colors"
              >
                Send
              </button>
            </div>

            {/* Autocomplete dropdown */}
            <Show when={showAutocomplete()}>
              <div class="absolute bottom-full left-0 mb-2 bg-gray-800 border border-gray-700 rounded-lg shadow-lg max-w-md w-full">
                <div class="p-2 text-xs text-gray-500 border-b border-gray-700">
                  {autocompleteType() === "agent" ? "Agents" : "Projects"}
                </div>
                <For each={autocompleteOptions()}>
                  {(option) => (
                    <button
                      onClick={() => selectAutocomplete(option)}
                      class="w-full text-left px-4 py-2 text-sm text-white hover:bg-gray-700 transition-colors"
                    >
                      {autocompleteType() === "agent" ? "@" : "#"}{option}
                    </button>
                  )}
                </For>
              </div>
            </Show>
          </div>

          {/* Show detected mentions */}
          <Show when={mentions().length > 0}>
            <div class="mt-2 text-sm text-gray-400">
              Routing to: <For each={mentions()}>{(m, idx) => (
                <span class="text-cyan-400">
                  @{m}{idx() < mentions().length - 1 ? ", " : ""}
                </span>
              )}</For>
            </div>
          </Show>
        </div>
      </div>

      {/* Right: Debug / Activity drawer */}
      <div class="w-96 border-l border-gray-800 p-4 overflow-y-auto bg-gray-950">
        <div class="flex items-center justify-between mb-3">
          <h3 class="text-lg font-semibold text-white">{showDebug() ? "Message Flow Debug" : "Activity Feed"}</h3>
          <button
            onClick={() => setShowDebug(!showDebug())}
            class="text-xs px-2 py-1 rounded bg-gray-800 text-gray-300 hover:bg-gray-700"
            title="Toggle debug panel"
          >
            {showDebug() ? "Activity" : "Debug"}
          </button>
        </div>

        <Show when={showDebug()}>
          {/* Inbox poll status */}
          <div class="mb-4 bg-gray-900 rounded-lg p-3 border border-gray-800">
            <div class="flex items-center justify-between text-xs mb-1">
              <span class="text-gray-400">GET /api/org/messages?agent=ceo</span>
              <span class={pollStatus().ok ? "text-green-400" : "text-red-400"}>
                {pollStatus().ok ? "OK" : "ERR"}
              </span>
            </div>
            <div class="text-xs text-gray-500">last poll: {pollStatus().at} • {pollStatus().msg}</div>
          </div>

          {/* Last send-message response */}
          <div class="mb-4 bg-gray-900 rounded-lg p-3 border border-gray-800">
            <div class="text-xs text-gray-400 mb-2">Last POST /api/org/send-message</div>
            <Show when={lastSendError()}>
              <pre class="text-xs text-red-300 whitespace-pre-wrap break-words">{lastSendError()}</pre>
            </Show>
            <Show when={!lastSendError() && lastSendResponse()}>
              <pre class="text-xs text-green-300 whitespace-pre-wrap break-words">{JSON.stringify(lastSendResponse(), null, 2)}</pre>
            </Show>
            <Show when={!lastSendError() && !lastSendResponse()}>
              <div class="text-xs text-gray-600 italic">no requests yet</div>
            </Show>
          </div>

          {/* Recipient online status */}
          <div class="mb-4 bg-gray-900 rounded-lg p-3 border border-gray-800">
            <div class="text-xs text-gray-400 mb-2">Recipient status (from REST org chart)</div>
            <Show when={lastSendResponse() && lastSendResponse()!.routed_to.length > 0} fallback={
              <div class="text-xs text-gray-600 italic">send a message to see routing</div>
            }>
              <For each={lastSendResponse()!.routed_to}>
                {(name) => {
                  const node = nodes().find(n => n.name === name);
                  const online = node?.status === "online";
                  return (
                    <div class="flex items-center justify-between text-xs py-0.5">
                      <span class="text-gray-300">{name}</span>
                      <span class={online ? "text-green-400" : "text-gray-500"}>
                        {online ? "● online" : "○ offline (no reply expected)"}
                      </span>
                    </div>
                  );
                }}
              </For>
            </Show>
          </div>

          {/* Real DMs to ceo (raw) */}
          <div class="mb-4 bg-gray-900 rounded-lg p-3 border border-gray-800">
            <div class="text-xs text-gray-400 mb-2">DMs to ceo (real, from agent_messages)</div>
            <Show when={polledMessages().length > 0} fallback={
              <div class="text-xs text-gray-600 italic">no DMs yet — agents have not replied</div>
            }>
              <div class="space-y-2 max-h-64 overflow-y-auto">
                <For each={polledMessages()}>
                  {(m) => (
                    <div class="text-xs border-l-2 border-cyan-700 pl-2">
                      <div class="text-gray-400">
                        <span class="text-cyan-400">{m.from}</span>
                        <span class="text-gray-600"> → </span>
                        <span class="text-gray-300">{m.to ?? "(channel)"}</span>
                        <span class="text-gray-600"> · {m.timestamp.split("T")[1]?.slice(0, 8) ?? m.timestamp}</span>
                      </div>
                      <div class="text-gray-200 break-words"><ReplyBody content={m.content} /></div>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </div>

          {/* Wire log */}
          <div class="bg-gray-900 rounded-lg p-3 border border-gray-800">
            <div class="text-xs text-gray-400 mb-2">Wire log</div>
            <div class="space-y-0.5 max-h-48 overflow-y-auto font-mono">
              <Show when={debugLog().length > 0} fallback={
                <div class="text-xs text-gray-600 italic">no events yet</div>
              }>
                <For each={debugLog().slice().reverse()}>
                  {(entry) => (
                    <div class="text-xs">
                      <span class="text-gray-600">{entry.ts}</span>
                      <span class={`mx-2 ${
                        entry.tag === "ok" ? "text-green-400" :
                        entry.tag.endsWith("-err") ? "text-red-400" :
                        entry.tag === "send" ? "text-cyan-400" :
                        "text-gray-500"
                      }`}>{entry.tag}</span>
                      <span class="text-gray-300">{entry.text}</span>
                    </div>
                  )}
                </For>
              </Show>
            </div>
          </div>
        </Show>

        <Show when={!showDebug()}>
          <div class="space-y-3">
            <For each={animations()}>
              {(anim) => (
                <div class="bg-gray-800 rounded-lg p-3">
                  <div class="flex items-center justify-between mb-2">
                    <div class="text-sm text-gray-300">{anim.from} → {anim.to}</div>
                    <div class={`text-xs px-2 py-0.5 rounded ${
                      anim.status === "routing" ? "bg-yellow-900 text-yellow-300" :
                      anim.status === "processing" ? "bg-blue-900 text-blue-300" :
                      "bg-green-900 text-green-300"
                    }`}>{anim.status}</div>
                  </div>
                  <div class="w-full bg-gray-700 rounded-full h-1.5">
                    <div class="bg-cyan-500 h-1.5 rounded-full transition-all duration-300" style={`width: ${anim.progress}%`}></div>
                  </div>
                </div>
              )}
            </For>
            <Show when={animations().length === 0 && messages().length === 0}>
              <div class="text-gray-500 text-sm text-center py-8">
                No activity yet.<br />Start by messaging your team.
              </div>
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
};

export default TeamDashboard;
