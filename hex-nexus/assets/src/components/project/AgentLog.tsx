/**
 * AgentLog.tsx — Agent Inspector with live status, controls, and memory.
 *
 * Shows agent status, tasks, heartbeat, and scoped memory.
 * Controls: kill, restart, reassign.
 * Data from SpacetimeDB agent-registry + hexflo-coordination subscriptions.
 */
import { Component, Show, For, createMemo, createSignal } from "solid-js";
import { registryAgents, agentHeartbeats, swarmTasks, hexfloMemory } from "../../stores/connection";
import { restClient } from "../../services/rest-client";

const AgentLog: Component<{ agentId: string }> = (props) => {
  const agent = createMemo(() =>
    registryAgents().find((a: any) =>
      (a.id ?? a.agent_id ?? "") === props.agentId
    )
  );

  const heartbeat = createMemo(() =>
    agentHeartbeats().find((h: any) =>
      (h.agent_id ?? "") === props.agentId
    )
  );

  const assignedTasks = createMemo(() =>
    swarmTasks().filter((t: any) =>
      (t.assigned_to ?? t.agent_id ?? "") === props.agentId
    )
  );

  // Agent-scoped memory from SpacetimeDB subscription (no fetch needed)
  const agentMemory = createMemo(() =>
    hexfloMemory().filter((m: any) =>
      (m.scope ?? m.memory_scope ?? "").includes(`agent:${props.agentId}`)
    )
  );

  const [killing, setKilling] = createSignal(false);

  function statusColor(status: string): string {
    if (status === "active" || status === "online") return "bg-green-500";
    if (status === "stale" || status === "warning") return "bg-yellow-500";
    return "bg-red-500";
  }

  function heartbeatAge(): string {
    const hb = heartbeat();
    if (!hb?.timestamp) return "—";
    const diff = Date.now() - new Date(hb.timestamp).getTime();
    if (diff < 60_000) return `${Math.floor(diff / 1000)}s ago`;
    if (diff < 3_600_000) return `${Math.floor(diff / 60_000)}m ago`;
    return `${Math.floor(diff / 3_600_000)}h ago`;
  }

  async function handleKill() {
    setKilling(true);
    try {
      await restClient.post(`/api/agents/${encodeURIComponent(props.agentId)}/kill`);
    } finally {
      setKilling(false);
    }
  }

  async function handleRestart() {
    const a = agent();
    if (!a) return;
    await handleKill();
    // Re-spawn with same config
    await restClient.post("/api/agents/spawn", {
      projectDir: a.project ?? a.project_dir ?? ".",
      agentName: a.name ?? a.agent_name,
    });
  }

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-4">
      <Show
        when={agent()}
        fallback={
          <div class="flex flex-1 items-center justify-center">
            <p class="text-sm text-gray-300">Agent not found: {props.agentId}</p>
          </div>
        }
      >
        {(a) => (
          <>
            {/* Agent header with controls */}
            <div class="mb-4 flex items-center justify-between">
              <div class="flex items-center gap-3">
                <span class={`h-3 w-3 rounded-full ${statusColor(a().status ?? a().state ?? "")}`} />
                <div>
                  <h3 class="text-sm font-semibold text-gray-100">
                    {a().name ?? a().agent_name ?? "unnamed"}
                  </h3>
                  <p class="text-[10px] text-gray-300">
                    {a().project ?? ""} — {a().status ?? a().state ?? "unknown"}
                  </p>
                </div>
              </div>

              {/* Controls */}
              <div class="flex items-center gap-1.5">
                <button
                  class="rounded border border-gray-700 px-2.5 py-1 text-[10px] text-gray-300 transition-colors hover:border-cyan-600 hover:text-cyan-300"
                  onClick={handleRestart}
                  title="Restart agent"
                >
                  Restart
                </button>
                <button
                  class="rounded border border-gray-700 px-2.5 py-1 text-[10px] text-gray-300 transition-colors hover:border-red-600 hover:text-red-400"
                  onClick={handleKill}
                  disabled={killing()}
                  title="Kill agent"
                >
                  {killing() ? "..." : "Kill"}
                </button>
              </div>
            </div>

            {/* Stats */}
            <div class="mb-4 grid grid-cols-4 gap-3">
              <StatBox label="Tasks" value={assignedTasks().length} />
              <StatBox label="Heartbeat" value={heartbeatAge()} />
              <StatBox label="Status" value={a().status ?? a().state ?? "—"} />
              <StatBox label="Memory" value={agentMemory().length} />
            </div>

            {/* Tasks section */}
            <Section title="Assigned Tasks" count={assignedTasks().length}>
              <Show
                when={assignedTasks().length > 0}
                fallback={<p class="text-xs text-gray-300">No tasks assigned</p>}
              >
                <div class="space-y-1">
                  <For each={assignedTasks()}>
                    {(t) => (
                      <div class="flex items-center justify-between rounded border border-gray-800 bg-gray-900/60 px-3 py-2 text-xs">
                        <span class="truncate text-gray-300">{t.title ?? "Untitled"}</span>
                        <span
                          class="ml-2 shrink-0 rounded px-1.5 py-0.5 text-[10px]"
                          classList={{
                            "bg-cyan-900/40 text-cyan-400": t.status === "in_progress",
                            "bg-green-900/40 text-green-400": t.status === "completed",
                            "bg-red-900/40 text-red-400": t.status === "failed",
                            "bg-gray-800 text-gray-300": !["in_progress", "completed", "failed"].includes(t.status ?? ""),
                          }}
                        >
                          {t.status ?? "pending"}
                        </span>
                      </div>
                    )}
                  </For>
                </div>
              </Show>
            </Section>

            {/* Memory section */}
            <Section title="Scoped Memory" count={agentMemory().length}>
              <Show
                when={agentMemory().length > 0}
                fallback={<p class="text-xs text-gray-300">No scoped memory</p>}
              >
                <div class="space-y-1">
                  <For each={agentMemory()}>
                    {(m) => (
                      <div class="rounded border border-gray-800 bg-gray-900/60 px-3 py-2 text-xs">
                        <div class="flex items-center justify-between">
                          <span class="font-mono text-cyan-300">{m.key ?? "?"}</span>
                          <span class="text-[10px] text-gray-300">{m.scope ?? ""}</span>
                        </div>
                        <p class="mt-1 truncate text-gray-300">{m.value ?? ""}</p>
                      </div>
                    )}
                  </For>
                </div>
              </Show>
            </Section>
          </>
        )}
      </Show>
    </div>
  );
};

const Section: Component<{ title: string; count: number; children: any }> = (props) => (
  <div class="mb-4">
    <div class="mb-2 flex items-center gap-2">
      <h4 class="text-[11px] font-semibold uppercase tracking-wider text-gray-300">
        {props.title}
      </h4>
      <span class="rounded bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-300">
        {props.count}
      </span>
    </div>
    {props.children}
  </div>
);

const StatBox: Component<{ label: string; value: string | number }> = (props) => (
  <div class="rounded border border-gray-800 bg-gray-900/40 px-3 py-2">
    <p class="text-[10px] text-gray-300">{props.label}</p>
    <p class="text-sm font-medium text-gray-100">{props.value}</p>
  </div>
);

export default AgentLog;
