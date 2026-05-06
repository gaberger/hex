/**
 * TeamDashboard.tsx — Interactive CEO dashboard with animated org communication
 *
 * Visual features:
 * - Live org chart with pulsing active agents
 * - Chat interface with @mention autocomplete
 * - Animated message routing visualization
 * - Real-time delegation chain display
 */

import { Component, For, Show, createSignal, onMount, createEffect, createMemo } from "solid-js";
import { restClient } from "../../services/rest-client";
import { orgCommsClient, type SendMessageResponse, type ConversationMessage } from "../../services/org-comms";
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

  onMount(async () => {
    try {
      const response = await restClient.get("/api/org/personas");
      setNodes(response.personas || response.nodes || []);
      setLoading(false);

      // Mark agents with status "online" as active
      const active = (response.personas || [])
        .filter((p: AgentOrgNode) => p.status === "online")
        .map((p: AgentOrgNode) => p.name);
      setActiveAgents(active);
    } catch (err) {
      console.error("Failed to load org chart:", err);
      setLoading(false);
    }
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
      to: mentionList.length > 0 ? mentionList[0] : "system",
      content: text,
      timestamp: new Date().toISOString(),
      status: "sent",
    };
    setMessages(prev => [...prev, userMsg]);
    setMessage("");

    if (mentionList.length === 0) {
      // No mentions - just echo
      return;
    }

    try {
      // Send through org routing
      const response = await orgCommsClient.sendMessage({
        from: "ceo",
        content: text,
      });

      // Animate routing to each agent
      response.routed_to.forEach((agent, idx) => {
        setTimeout(() => {
          setActiveAgents(prev => [...prev, agent]);

          // Create animation
          const anim: MessageAnimation = {
            id: `anim-${Date.now()}-${Math.random().toString(36).substring(2, 9)}`,
            from: "ceo",
            to: agent,
            progress: 0,
            status: "routing",
          };
          setAnimations(prev => [...prev, anim]);

          // Animate progress
          let progress = 0;
          const interval = setInterval(() => {
            progress += 5;
            setAnimations(prev =>
              prev.map(a => a.id === anim.id ? { ...a, progress } : a)
            );

            if (progress >= 100) {
              clearInterval(interval);
              setAnimations(prev =>
                prev.map(a => a.id === anim.id ? { ...a, status: "processing" } : a)
              );

              // Simulate agent "thinking"
              setTimeout(() => {
                setAnimations(prev =>
                  prev.map(a => a.id === anim.id ? { ...a, status: "completed" } : a)
                );
                setActiveAgents(prev => prev.filter(a => a !== agent));

                // Add agent response (placeholder)
                const agentMsg: ConversationMessage = {
                  id: `msg-${Date.now()}-${Math.random().toString(36).substring(2, 9)}`,
                  from: agent,
                  to: "ceo",
                  content: `[${agent}] Acknowledged. Processing your request...`,
                  timestamp: new Date().toISOString(),
                  status: "completed",
                };
                setMessages(prev => [...prev, agentMsg]);

                // Remove animation after 2s
                setTimeout(() => {
                  setAnimations(prev => prev.filter(a => a.id !== anim.id));
                }, 2000);
              }, 2000);
            }
          }, 30);
        }, idx * 200); // Stagger animations
      });
    } catch (err) {
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
              <div class="text-xs text-gray-500 uppercase tracking-wider mb-2">Individual Contributors ({nodes().filter(n => n.tier === "ic").length})</div>
              <div class="grid grid-cols-2 gap-2">
                <For each={nodes().filter(n => n.tier === "ic").slice(0, 6)}>
                  {(agent) => <AgentCard agent={agent} />}
                </For>
              </div>
              <Show when={nodes().filter(n => n.tier === "ic").length > 6}>
                <div class="text-gray-500 text-sm mt-2">
                  +{nodes().filter(n => n.tier === "ic").length - 6} more ICs
                </div>
              </Show>
            </div>
          </div>
        </Show>
      </div>

      {/* Center: Chat Interface */}
      <div class="flex-1 flex flex-col">
        {/* Header */}
        <div class="p-6 border-b border-gray-800">
          <h1 class="text-2xl font-bold text-white">CEO Command Center</h1>
          <p class="text-gray-400 mt-1">
            Use @mentions to route messages: <span class="text-cyan-400">@cto</span>, <span class="text-purple-400">@cpo</span>, <span class="text-blue-400">@coo</span>.
            No @mentions → all executives (board meeting). Use <span class="text-green-400">#project-name</span> to scope.
          </p>
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
                      {new Date(msg.timestamp).toLocaleTimeString()}
                    </div>
                  </div>
                  <div class="text-sm">{msg.content}</div>
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

      {/* Right: Activity Feed */}
      <div class="w-80 border-l border-gray-800 p-6 overflow-y-auto">
        <h3 class="text-lg font-semibold text-white mb-4">Activity Feed</h3>

        <div class="space-y-3">
          <For each={animations()}>
            {(anim) => (
              <div class="bg-gray-800 rounded-lg p-3">
                <div class="flex items-center justify-between mb-2">
                  <div class="text-sm text-gray-300">
                    {anim.from} → {anim.to}
                  </div>
                  <div class={`
                    text-xs px-2 py-0.5 rounded
                    ${anim.status === "routing" ? "bg-yellow-900 text-yellow-300" :
                      anim.status === "processing" ? "bg-blue-900 text-blue-300" :
                      "bg-green-900 text-green-300"}
                  `}>
                    {anim.status}
                  </div>
                </div>

                {/* Progress bar */}
                <div class="w-full bg-gray-700 rounded-full h-1.5">
                  <div
                    class="bg-cyan-500 h-1.5 rounded-full transition-all duration-300"
                    style={`width: ${anim.progress}%`}
                  ></div>
                </div>
              </div>
            )}
          </For>

          <Show when={animations().length === 0 && messages().length === 0}>
            <div class="text-gray-500 text-sm text-center py-8">
              No activity yet.<br />
              Start by messaging your team.
            </div>
          </Show>
        </div>
      </div>
    </div>
  );
};

export default TeamDashboard;
