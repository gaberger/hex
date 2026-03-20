/**
 * SwarmHeader.tsx — Phase indicator, progress bar, and agent count.
 *
 * Shows the current phase of the hex feature development lifecycle
 * and overall task completion percentage.
 */
import { Component, For, createMemo } from "solid-js";

const PHASES = ["SPECS", "PLAN", "CODE", "VALIDATE", "INTEGRATE"] as const;

interface SwarmInfo {
  name: string;
  topology: string;
  status: string;
  tasks: { status: string }[];
  agents: number;
}

function detectPhase(tasks: { status: string }[]): number {
  if (tasks.length === 0) return 0;
  const completed = tasks.filter(t => t.status === "completed").length;
  const ratio = completed / tasks.length;
  if (ratio >= 1) return 4; // INTEGRATE
  if (ratio >= 0.7) return 3; // VALIDATE
  if (ratio >= 0.2) return 2; // CODE
  if (ratio > 0) return 1; // PLAN
  return 0; // SPECS
}

const SwarmHeader: Component<{ swarm: SwarmInfo }> = (props) => {
  const completedCount = createMemo(
    () => props.swarm.tasks.filter(t => t.status === "completed").length
  );
  const progress = createMemo(
    () => props.swarm.tasks.length > 0
      ? Math.round((completedCount() / props.swarm.tasks.length) * 100)
      : 0
  );
  const currentPhase = createMemo(() => detectPhase(props.swarm.tasks));

  return (
    <div class="mb-4 space-y-3">
      {/* Title row */}
      <div class="flex items-center justify-between">
        <div class="flex items-center gap-3">
          <h3 class="text-sm font-semibold text-gray-100">{props.swarm.name}</h3>
          <span class="rounded bg-cyan-900/40 px-2 py-0.5 text-[10px] text-cyan-400">
            {props.swarm.topology}
          </span>
        </div>
        <div class="flex items-center gap-4 text-[10px] text-gray-300">
          <span>{props.swarm.agents} agents</span>
          <span>{completedCount()}/{props.swarm.tasks.length} tasks</span>
          <span class="font-bold text-gray-100">{progress()}%</span>
        </div>
      </div>

      {/* Phase indicator */}
      <div class="flex items-center gap-1">
        <For each={PHASES}>
          {(phase, i) => (
            <div class="flex items-center">
              <div
                class="flex items-center justify-center rounded-full px-3 py-1 text-[10px] font-semibold transition-colors"
                classList={{
                  "bg-cyan-600 text-white": i() === currentPhase(),
                  "bg-green-900/40 text-green-400": i() < currentPhase(),
                  "bg-gray-800 text-gray-300": i() > currentPhase(),
                }}
              >
                {phase}
              </div>
              {i() < PHASES.length - 1 && (
                <div
                  class="mx-1 h-px w-6"
                  classList={{
                    "bg-green-600": i() < currentPhase(),
                    "bg-gray-700": i() >= currentPhase(),
                  }}
                />
              )}
            </div>
          )}
        </For>
      </div>

      {/* Progress bar */}
      <div class="h-1.5 rounded-full bg-gray-800">
        <div
          class="h-full rounded-full transition-all duration-500"
          classList={{
            "bg-cyan-500": progress() < 100,
            "bg-green-500": progress() >= 100,
          }}
          style={{ width: `${progress()}%` }}
        />
      </div>
    </div>
  );
};

export default SwarmHeader;
