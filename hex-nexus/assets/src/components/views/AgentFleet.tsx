/**
 * AgentFleet.tsx — View showing all registered agents split into LOCAL and
 * REMOTE sections with status, role, task, and uptime details.
 *
 * Data sources: SpacetimeDB subscriptions via connection stores.
 */
import { Component, For, Show, createMemo, createSignal } from "solid-js";
import { registryAgents, swarmAgents, swarmTasks } from "../../stores/connection";
import { projects } from "../../stores/projects";
import { setSpawnDialogOpen } from "../../stores/ui";
import { openPane } from "../../stores/panes";
import { addToast } from "../../stores/toast";
import { restClient } from "../../services/rest-client";
import { matchesProject, getEntityProjectRef, getAgentProjectDir } from "../../utils/project-match";
import { activeProjectId } from "../../stores/router";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function statusDotColor(status: string): string {
  if (status === "active" || status === "online") return "bg-green-500";
  if (status === "idle") return "bg-yellow-500";
  if (status === "stale" || status === "warning") return "bg-yellow-500";
  if (status === "dead" || status === "offline" || status === "error") return "bg-red-500";
  return "bg-gray-500";
}

function statusBadgeBg(status: string): string {
  if (status === "active" || status === "online") return "bg-green-900/30 text-green-400";
  if (status === "idle") return "bg-yellow-900/30 text-yellow-400";
  if (status === "stale") return "bg-yellow-900/30 text-yellow-400";
  if (status === "dead" || status === "offline") return "bg-red-900/30 text-red-400";
  return "bg-gray-800 text-gray-400";
}

function formatUptime(startedAt: string | undefined): string {
  if (!startedAt) return "--";
  const diff = Date.now() - new Date(startedAt).getTime();
  if (diff < 0) return "--";
  const secs = Math.floor(diff / 1000);
  if (secs < 60) return `${secs}s`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ${mins % 60}m`;
  return `${Math.floor(hrs / 24)}d ${hrs % 24}h`;
}

// ---------------------------------------------------------------------------
// Main component
// ---------------------------------------------------------------------------

const AgentFleet: Component = () => {
  const localAgents = createMemo(() => {
    return registryAgents().filter(
      (a: any) => !a.host && !a.remote && !a.transport
    );
  });

  const remoteAgents = createMemo(() => {
    return registryAgents().filter(
      (a: any) => a.host || a.remote || a.transport
    );
  });

  const totalCount = createMemo(() => registryAgents().length);

  /** Group local agents by project. Returns null when grouping adds no value. */
  const agentsByProject = createMemo(() => {
    const agents = localAgents();
    const groups = new Map<string, any[]>();
    for (const agent of agents) {
      const proj = agent.project ?? agent.projectId ?? agent.project_id ?? "unassigned";
      if (!groups.has(proj)) groups.set(proj, []);
      groups.get(proj)!.push(agent);
    }
    // Skip grouping if all agents belong to a single bucket
    if (groups.size <= 1) return null;
    return groups;
  });

  function agentProject(agent: any): string {
    const ref = getEntityProjectRef(agent);
    const dir = getAgentProjectDir(agent);
    if (!ref && !dir) return "--";
    const proj = projects().find((p) =>
      matchesProject(ref, p.id) || (dir && matchesProject(dir, p.id)),
    );
    return proj?.name ?? (ref || dir || "--");
  }

  function agentTask(agent: any): string | null {
    const task = swarmTasks().find(
      (t: any) =>
        (t.assigned_to ?? t.agent_id ?? "") === (agent.id ?? agent.agent_id ?? "") &&
        (t.status === "in_progress" || t.status === "active")
    );
    return task?.title ?? null;
  }

  function agentHeartbeatAge(agent: any): string | null {
    // ADR-058: heartbeat is inline on hex_agent.lastHeartbeat
    const ts = agent.lastHeartbeat ?? agent.last_heartbeat;
    if (!ts) return null;
    const diff = Date.now() - new Date(ts).getTime();
    if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
    if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
    return `${Math.floor(diff / 3_600_000)}h ago`;
  }

  function handleAgentClick(agent: any) {
    openPane("agent-log", agent.name ?? agent.agent_name ?? "agent", {
      agentId: agent.id ?? agent.agent_id,
    });
  }

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-6">
      {/* Header */}
      <div class="mb-6 flex items-center justify-between">
        <div>
          <h2 class="text-[22px] font-bold text-gray-100">Agent Fleet</h2>
          <p class="mt-0.5 text-xs text-gray-400">
            {totalCount()} agent{totalCount() !== 1 ? "s" : ""} registered
          </p>
        </div>

        <div class="flex items-center gap-3">
          <button
            class="rounded-lg border border-gray-700 bg-gray-900 px-3 py-1.5 text-xs font-medium text-gray-300 transition-colors hover:border-gray-600 hover:text-gray-100"
            onClick={() => addToast("info", "Run: hex agent connect <host:port> to connect a remote agent")}
          >
            Connect Remote
          </button>
          <button
            class="rounded-lg bg-cyan-600 px-3 py-1.5 text-xs font-medium text-white transition-colors hover:bg-cyan-500"
            onClick={() => setSpawnDialogOpen(true)}
          >
            Spawn Agent
          </button>
        </div>
      </div>

      {/* LOCAL AGENTS */}
      <section class="mb-8">
        <h3 class="mb-3 text-[12px] font-semibold uppercase tracking-wider text-gray-500">
          Local Agents
          <span class="ml-2 text-gray-600">({localAgents().length})</span>
        </h3>

        <Show
          when={localAgents().length > 0}
          fallback={
            <div class="rounded-xl border border-dashed border-gray-800 bg-gray-900/30 px-6 py-8 text-center">
              <p class="text-sm text-gray-400">No local agents running</p>
              <p class="mt-1 text-[11px] text-gray-500">
                Click "Spawn Agent" to start one, or run{" "}
                <code class="rounded bg-gray-800 px-1 py-0.5 font-mono text-[10px] text-cyan-300">
                  hex agent spawn
                </code>
              </p>
            </div>
          }
        >
          <Show
            when={agentsByProject()}
            fallback={
              <div class="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3">
                <For each={localAgents()}>
                  {(agent) => <AgentCard agent={agent} agentProject={agentProject} agentTask={agentTask} agentHeartbeatAge={agentHeartbeatAge} onClick={handleAgentClick} />}
                </For>
              </div>
            }
          >
            {(groups) => (
              <For each={[...groups().entries()]}>
                {([projectName, agents]) => (
                  <div>
                    <h4 class="mb-2 text-xs font-bold uppercase tracking-wider text-gray-500">
                      {projectName} ({agents.length})
                    </h4>
                    <div class="mb-6 grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
                      <For each={agents}>
                        {(agent) => <AgentCard agent={agent} agentProject={agentProject} agentTask={agentTask} agentHeartbeatAge={agentHeartbeatAge} onClick={handleAgentClick} />}
                      </For>
                    </div>
                  </div>
                )}
              </For>
            )}
          </Show>
        </Show>
      </section>

      {/* REMOTE AGENTS */}
      <section>
        <h3 class="mb-3 text-[12px] font-semibold uppercase tracking-wider text-gray-500">
          Remote Agents
          <span class="ml-2 text-gray-600">({remoteAgents().length})</span>
        </h3>

        <Show
          when={remoteAgents().length > 0}
          fallback={
            <div class="rounded-xl border border-dashed border-gray-800 bg-gray-900/30 px-6 py-8 text-center">
              <p class="text-sm text-gray-400">No remote agents connected</p>
              <p class="mt-1 text-[11px] text-gray-500">
                Remote agents from other machines will appear here when connected via fleet protocol.
              </p>
            </div>
          }
        >
          <div class="grid grid-cols-1 gap-3 md:grid-cols-2 lg:grid-cols-3">
            <For each={remoteAgents()}>
              {(agent) => {
                const status = () => agent.status ?? "offline";
                const name = () => {
                  const host = agent.host ?? "remote";
                  const n = agent.name ?? agent.agent_name ?? "agent";
                  return `${host}:${n}`;
                };

                return (
                  <div class="flex flex-col gap-2.5 rounded-xl border border-gray-800 bg-gray-900 p-4">
                    {/* Top row */}
                    <div class="flex items-center justify-between">
                      <div class="flex items-center gap-2">
                        <span class={`h-2.5 w-2.5 shrink-0 rounded-full ${statusDotColor(status())}`} role="img" aria-label={status()} /><span class="sr-only">{status()}</span>
                        <span class="truncate font-mono text-xs font-semibold text-gray-100">
                          {name()}
                        </span>
                      </div>
                      <span
                        class={`rounded-full px-2 py-0.5 text-[10px] font-medium ${statusBadgeBg(status())}`}
                      >
                        {status()}
                      </span>
                    </div>

                    {/* Connection details */}
                    <div class="grid grid-cols-2 gap-x-4 gap-y-1 text-[12px]">
                      <DetailRow label="Host" value={agent.host ?? "--"} />
                      <DetailRow label="Transport" value={agent.transport ?? "ssh"} />
                      <DetailRow label="Inference" value={agent.inference ?? "local"} />
                      <DetailRow label="Latency" value={agent.latency ? `${agent.latency}ms` : "--"} />
                    </div>
                  </div>
                );
              }}
            </For>
          </div>
        </Show>
      </section>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

const AgentCard: Component<{
  agent: any;
  agentProject: (a: any) => string;
  agentTask: (a: any) => string | null;
  agentHeartbeatAge: (a: any) => string | null;
  onClick: (a: any) => void;
}> = (props) => {
  const agent = props.agent;
  const status = () => agent.status ?? "idle";
  const name = () => agent.name ?? agent.agent_name ?? "unnamed";
  const role = () => agent.agentType ?? agent.agent_type ?? agent.role ?? (name().startsWith("claude-code") ? "claude-code" : "--");
  const model = () => agent.model ?? "--";
  const uptime = () => formatUptime(agent.startedAt ?? agent.started_at ?? agent.created_at ?? agent.connectedAt ?? agent.connected_at);
  const task = () => props.agentTask(agent);
  const [confirming, setConfirming] = createSignal(false);

  async function handleTerminate(e: MouseEvent) {
    e.stopPropagation();
    const agentId = agent.id ?? agent.agent_id;
    if (!confirming()) {
      setConfirming(true);
      setTimeout(() => setConfirming(false), 3000);
      return;
    }
    setConfirming(false);
    try {
      await restClient.delete(`/api/agents/${encodeURIComponent(agentId)}`);
      addToast("success", `Agent "${name()}" terminated`);
    } catch (err: any) {
      addToast("error", `Failed to terminate agent: ${err.message ?? err}`);
    }
  }

  return (
    <button
      class="group flex flex-col gap-2.5 rounded-xl border border-gray-800 bg-gray-900 p-4 text-left transition-all hover:border-gray-700 hover:bg-[#111827] focus:outline-none focus:ring-1 focus:ring-cyan-500/50"
      onClick={() => props.onClick(agent)}
    >
      {/* Top row: dot + name + status badge + kill button */}
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-2">
          <span class={`h-2.5 w-2.5 shrink-0 rounded-full ${statusDotColor(status())}`} role="img" aria-label={status()} /><span class="sr-only">{status()}</span>
          <span class="truncate font-mono text-xs font-semibold text-gray-100">
            {name()}
          </span>
        </div>
        <div class="flex items-center gap-1.5">
          <span
            class={`rounded-full px-2 py-0.5 text-[10px] font-medium ${statusBadgeBg(status())}`}
          >
            {status()}
          </span>
          <span
            role="button"
            title={confirming() ? "Click again to confirm" : "Terminate agent"}
            class={`flex h-5 w-5 items-center justify-center rounded transition-colors ${
              confirming()
                ? "bg-red-600 text-white"
                : "text-gray-600 opacity-0 hover:bg-red-900/40 hover:text-red-400 group-hover:opacity-100"
            }`}
            onClick={handleTerminate}
          >
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="currentColor" class="h-3 w-3">
              <path d="M5.28 4.22a.75.75 0 0 0-1.06 1.06L6.94 8l-2.72 2.72a.75.75 0 1 0 1.06 1.06L8 9.06l2.72 2.72a.75.75 0 1 0 1.06-1.06L9.06 8l2.72-2.72a.75.75 0 0 0-1.06-1.06L8 6.94 5.28 4.22Z" />
            </svg>
          </span>
        </div>
      </div>

      {/* Details grid */}
      <div class="grid grid-cols-2 gap-x-4 gap-y-1 text-[12px]">
        <DetailRow label="Role" value={role()} />
        <DetailRow label="Project" value={props.agentProject(agent)} />
        <DetailRow label="Uptime" value={uptime()} />
        <DetailRow label="Model" value={model()} />
      </div>

      {/* Current task */}
      <Show when={task()}>
        <div class="flex items-center gap-2 rounded-lg bg-cyan-900/20 px-2.5 py-1.5">
          <div class="h-1.5 w-1.5 animate-pulse rounded-full bg-cyan-400" />
          <span class="truncate text-[11px] text-cyan-300">
            {task()}
          </span>
        </div>
      </Show>

      {/* Heartbeat */}
      <Show when={props.agentHeartbeatAge(agent)}>
        <p class="text-[10px] text-gray-500">
          Heartbeat: {props.agentHeartbeatAge(agent)}
        </p>
      </Show>
    </button>
  );
};

const DetailRow: Component<{ label: string; value: string }> = (props) => (
  <div class="flex items-baseline gap-1.5">
    <span class="text-gray-500">{props.label}:</span>
    <span class="truncate text-gray-300">{props.value}</span>
  </div>
);

export default AgentFleet;
