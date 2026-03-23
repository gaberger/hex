/**
 * AgentList.tsx — Project-scoped agent list with heartbeat-based liveness.
 *
 * Matches hex CLI `hex agent list` behavior:
 *   - online:  heartbeat < 45s ago
 *   - stale:   heartbeat 45s–120s ago
 *   - dead:    heartbeat > 120s ago or no heartbeat
 *   - completed: status === "completed"
 *
 * By default shows only online/stale agents. Toggle reveals all.
 */
import { Component, For, Show, createMemo, createSignal } from "solid-js";
import {
  swarms,
  swarmAgents,
  swarmTasks,
  registryAgents,
} from "../../stores/connection";
import { navigate, route } from "../../stores/router";
import { entityBelongsToProject } from "../../utils/project-match";

// ── Heartbeat protocol (ADR-027) ─────────────────────────
const STALE_THRESHOLD_MS = 45_000;   // 45 seconds
const DEAD_THRESHOLD_MS = 120_000;   // 2 minutes

type LiveStatus = "online" | "stale" | "dead" | "completed" | "unknown";

function computeLiveStatus(agent: any, heartbeat: any): LiveStatus {
  const recordedStatus = (agent.status ?? "").toLowerCase();
  if (recordedStatus === "completed" || recordedStatus === "done") return "completed";

  if (!heartbeat) {
    // No heartbeat record — check agent's own timestamp
    const started = agent.started_at ?? agent.startedAt ?? agent.registered_at ?? "";
    if (!started) return "unknown";
    const age = Date.now() - new Date(started).getTime();
    if (age > DEAD_THRESHOLD_MS) return "dead";
    return "online"; // Recently started, heartbeat not yet expected
  }

  const lastSeen = heartbeat.last_seen ?? heartbeat.timestamp ?? "";
  if (!lastSeen) return "unknown";
  const age = Date.now() - new Date(lastSeen).getTime();
  if (age < STALE_THRESHOLD_MS) return "online";
  if (age < DEAD_THRESHOLD_MS) return "stale";
  return "dead";
}

function statusDotClass(status: LiveStatus): string {
  switch (status) {
    case "online": return "bg-green-400";
    case "stale": return "bg-yellow-400 animate-pulse";
    case "dead": return "bg-red-400";
    case "completed": return "bg-gray-500";
    default: return "bg-gray-600";
  }
}

function statusLabel(status: LiveStatus): string {
  switch (status) {
    case "online": return "online";
    case "stale": return "stale";
    case "dead": return "dead";
    case "completed": return "completed";
    default: return "unknown";
  }
}

function statusTextClass(status: LiveStatus): string {
  switch (status) {
    case "online": return "text-green-400";
    case "stale": return "text-yellow-400";
    case "dead": return "text-red-400";
    case "completed": return "text-gray-500";
    default: return "text-gray-600";
  }
}

function relativeTime(timestamp: string | undefined): string {
  if (!timestamp) return "--";
  const diff = Date.now() - new Date(timestamp).getTime();
  if (diff < 0) return "just now";
  if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
  return `${Math.floor(diff / 3_600_000)}h ago`;
}

// ── Component ─────────────────────────────────────────────

const AgentList: Component = () => {
  const projectId = () => (route() as any).projectId ?? "";
  const [showAll, setShowAll] = createSignal(false);

  // Find swarms belonging to this project
  const projectSwarms = createMemo(() =>
    swarms().filter((s: any) => entityBelongsToProject(s, projectId())),
  );

  const projectSwarmIds = createMemo(() =>
    new Set(projectSwarms().map((s: any) => s.id ?? s.swarm_id ?? "")),
  );

  // Agents from swarms belonging to this project
  const projectSwarmAgentList = createMemo(() =>
    swarmAgents().filter((a: any) =>
      projectSwarmIds().has(a.swarm_id ?? a.swarmId ?? ""),
    ),
  );

  // Global registry agents tied to this project
  const projectRegistryAgents = createMemo(() =>
    registryAgents().filter((a: any) => entityBelongsToProject(a, projectId())),
  );

  // Merge, deduplicate, and compute live status
  const allAgents = createMemo(() => {
    const seen = new Set<string>();
    const result: Array<{ agent: any; id: string; name: string; role: string; liveStatus: LiveStatus; heartbeat: any }> = [];

    for (const a of [...projectSwarmAgentList(), ...projectRegistryAgents()]) {
      const id = a.id ?? a.agent_id ?? "";
      if (!id || seen.has(id)) continue;
      seen.add(id);

      // ADR-058: heartbeat is inline on hex_agent.lastHeartbeat
      const hb = a.lastHeartbeat || a.last_heartbeat
        ? { last_seen: a.lastHeartbeat ?? a.last_heartbeat }
        : null;
      const liveStatus = computeLiveStatus(a, hb);

      result.push({
        agent: a,
        id,
        name: a.name ?? a.agent_name ?? a.agentName ?? "unnamed",
        role: a.role ?? a.agent_role ?? "",
        liveStatus,
        heartbeat: hb,
      });
    }

    // Sort: online first, then stale, then dead, then completed
    const order: Record<LiveStatus, number> = { online: 0, stale: 1, dead: 2, unknown: 3, completed: 4 };
    result.sort((a, b) => (order[a.liveStatus] ?? 9) - (order[b.liveStatus] ?? 9));

    return result;
  });

  // Filtered list (default: only online + stale)
  const visibleAgents = createMemo(() => {
    if (showAll()) return allAgents();
    return allAgents().filter((a) => a.liveStatus === "online" || a.liveStatus === "stale");
  });

  const onlineCount = createMemo(() =>
    allAgents().filter((a) => a.liveStatus === "online").length,
  );
  const staleCount = createMemo(() =>
    allAgents().filter((a) => a.liveStatus === "stale").length,
  );
  const totalCount = createMemo(() => allAgents().length);

  function taskCount(agentId: string): number {
    return swarmTasks().filter(
      (t: any) => (t.assigned_to ?? t.agent_id ?? "") === agentId,
    ).length;
  }

  function handleAgentClick(agentId: string) {
    navigate({
      page: "project-agent-detail",
      projectId: projectId(),
      agentId,
    });
  }

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-4">
      {/* Header */}
      <div class="mb-4 flex items-center justify-between">
        <div class="flex items-center gap-3">
          <h3 class="text-sm font-semibold text-gray-100">Agents</h3>
          <div class="flex items-center gap-2 text-[10px]">
            <span class="flex items-center gap-1">
              <span class="h-2 w-2 rounded-full bg-green-400" />
              <span class="text-green-400">{onlineCount()} online</span>
            </span>
            <Show when={staleCount() > 0}>
              <span class="flex items-center gap-1">
                <span class="h-2 w-2 rounded-full bg-yellow-400" />
                <span class="text-yellow-400">{staleCount()} stale</span>
              </span>
            </Show>
            <span class="text-gray-500">{totalCount()} total</span>
          </div>
        </div>
        <button
          class="rounded px-2 py-1 text-[10px] transition-colors"
          classList={{
            "bg-gray-800 text-gray-300": showAll(),
            "text-gray-500 hover:text-gray-300": !showAll(),
          }}
          onClick={() => setShowAll(!showAll())}
        >
          {showAll() ? "Show active only" : "Show all"}
        </button>
      </div>

      <Show
        when={visibleAgents().length > 0}
        fallback={
          <div class="flex flex-1 items-center justify-center">
            <p class="text-sm text-gray-400">
              {totalCount() > 0
                ? `No active agents (${totalCount()} completed/dead — click "Show all")`
                : "No agents for this project"}
            </p>
          </div>
        }
      >
        <div class="grid gap-3 sm:grid-cols-1 md:grid-cols-2 xl:grid-cols-3">
          <For each={visibleAgents()}>
            {(entry) => {
              const lastSeen = () =>
                relativeTime(entry.heartbeat?.last_seen ?? entry.heartbeat?.timestamp ?? entry.agent.started_at ?? entry.agent.startedAt);

              return (
                <button
                  class="flex flex-col gap-2 rounded-lg border p-3 text-left transition-colors"
                  classList={{
                    "border-green-800/40 bg-gray-900/50 hover:border-green-600/50": entry.liveStatus === "online",
                    "border-yellow-800/40 bg-gray-900/50 hover:border-yellow-600/50": entry.liveStatus === "stale",
                    "border-gray-800 bg-gray-900/30 hover:border-gray-600 opacity-60": entry.liveStatus === "dead" || entry.liveStatus === "completed",
                    "border-gray-800 bg-gray-900/50 hover:border-gray-600": entry.liveStatus === "unknown",
                  }}
                  onClick={() => handleAgentClick(entry.id)}
                >
                  {/* Top row: status + name + role */}
                  <div class="flex items-center gap-2">
                    <span class={`h-2.5 w-2.5 shrink-0 rounded-full ${statusDotClass(entry.liveStatus)}`} />
                    <span class="truncate text-sm font-medium text-gray-100">{entry.name}</span>
                    <span class={`ml-auto text-[9px] font-semibold uppercase ${statusTextClass(entry.liveStatus)}`}>
                      {statusLabel(entry.liveStatus)}
                    </span>
                  </div>

                  {/* Stats row */}
                  <div class="flex items-center gap-4 text-[10px] text-gray-400">
                    <Show when={entry.role}>
                      <span class="rounded bg-gray-800 px-1.5 py-0.5 text-gray-300">{entry.role}</span>
                    </Show>
                    <span>{taskCount(entry.id)} task{taskCount(entry.id) !== 1 ? "s" : ""}</span>
                    <span>seen {lastSeen()}</span>
                  </div>

                  {/* Worktree */}
                  <Show when={entry.agent.worktree ?? entry.agent.worktree_path}>
                    <p class="truncate font-mono text-[10px] text-gray-500">
                      {entry.agent.worktree ?? entry.agent.worktree_path}
                    </p>
                  </Show>
                </button>
              );
            }}
          </For>
        </div>
      </Show>
    </div>
  );
};

export default AgentList;
