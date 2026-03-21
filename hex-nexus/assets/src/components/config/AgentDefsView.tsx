import { Component, For } from 'solid-js';
import { addToast } from '../../stores/toast';
import { setSpawnDialogOpen } from '../../stores/ui';

interface AgentDef {
  name: string;
  role: string;
  model: string;
  desc: string;
  tools: string[];
  color: string;
}

const AGENT_DEFS: AgentDef[] = [
  { name: "hex-coder", role: "coder", model: "opus", desc: "Write code with TDD loop", tools: ["Read", "Write", "Edit", "Bash", "Grep"], color: "#4ade80" },
  { name: "planner", role: "planner", model: "opus", desc: "Decompose requirements into tasks", tools: ["Read", "Grep", "WebSearch"], color: "#60a5fa" },
  { name: "integrator", role: "integrator", model: "sonnet", desc: "Merge worktrees + integration tests", tools: ["Read", "Bash", "Grep"], color: "#22d3ee" },
  { name: "reviewer", role: "reviewer", model: "sonnet", desc: "Code review + quality checks", tools: ["Read", "Grep"], color: "#a78bfa" },
  { name: "tester", role: "tester", model: "haiku", desc: "Run tests + validation", tools: ["Read", "Bash"], color: "#eab308" },
];

const modelBadgeColor: Record<string, string> = {
  opus: "bg-purple-900/50 text-purple-300 border-purple-700/50",
  sonnet: "bg-blue-900/50 text-blue-300 border-blue-700/50",
  haiku: "bg-yellow-900/50 text-yellow-300 border-yellow-700/50",
};

const AgentDefsView: Component = () => {
  return (
    <div class="flex-1 overflow-auto p-6">
      {/* Header */}
      <div class="flex items-center justify-between mb-6">
        <div>
          <h2 class="text-xl font-bold text-gray-100">Agent Definitions</h2>
          <p class="mt-1 text-sm text-gray-400">
            Role definitions from <code class="text-xs font-mono text-gray-500">.claude/agents/</code> that configure agent capabilities.
          </p>
        </div>
        <button class="rounded-lg bg-gray-800 px-4 py-2 text-sm font-medium text-gray-300 hover:bg-gray-700 hover:text-gray-100 transition-colors border border-gray-700"
          onClick={() => setSpawnDialogOpen(true)}>
          Add Agent
        </button>
      </div>

      {/* Agent cards grid */}
      <div class="grid grid-cols-1 md:grid-cols-2 xl:grid-cols-3 gap-4">
        <For each={AGENT_DEFS}>
          {(agent) => (
            <div
              class="rounded-xl p-4 border"
              style={{
                "background-color": "#111827",
                "border-color": agent.color + "40",
              }}
            >
              {/* Name + colored dot */}
              <div class="flex items-center gap-2 mb-3">
                <span
                  class="h-2.5 w-2.5 rounded-full shrink-0"
                  style={{ "background-color": agent.color }}
                />
                <span class="font-bold font-mono text-sm text-gray-100">{agent.name}</span>
              </div>

              {/* Role + model badges */}
              <div class="flex items-center gap-2 mb-3">
                <span
                  class="inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium"
                  style={{
                    "background-color": agent.color + "18",
                    "border-color": agent.color + "40",
                    color: agent.color,
                  }}
                >
                  {agent.role}
                </span>
                <span
                  class={`inline-flex items-center rounded-md border px-2 py-0.5 text-xs font-medium ${modelBadgeColor[agent.model] ?? "bg-gray-800 text-gray-400 border-gray-700"}`}
                >
                  {agent.model}
                </span>
              </div>

              {/* Description */}
              <p class="text-sm text-gray-400 mb-4">{agent.desc}</p>

              {/* Tool chips */}
              <div class="flex flex-wrap gap-1.5 mb-4">
                <For each={agent.tools}>
                  {(tool) => (
                    <span class="rounded-full bg-gray-800 border border-gray-700 px-2.5 py-0.5 text-xs font-mono text-gray-400">
                      {tool}
                    </span>
                  )}
                </For>
              </div>

              {/* Edit button */}
              <button class="rounded-lg bg-gray-800 px-3 py-1.5 text-xs font-medium text-gray-400 hover:bg-gray-700 hover:text-gray-200 transition-colors border border-gray-700"
                onClick={() => addToast("info", `Edit agent definition: .claude/agents/${agent.name}.yml`)}>
                Edit
              </button>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

export default AgentDefsView;
