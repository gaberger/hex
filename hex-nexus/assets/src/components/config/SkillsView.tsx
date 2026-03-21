import { Component, For } from 'solid-js';

interface Skill {
  name: string;
  trigger: string;
  desc: string;
  source: string;
}

const SKILLS: Skill[] = [
  { name: "hex-scaffold", trigger: "/hex-scaffold", desc: "Scaffold a new hex project", source: ".claude/skills/" },
  { name: "hex-generate", trigger: "/hex-generate", desc: "Generate code within adapter boundary", source: ".claude/skills/" },
  { name: "hex-analyze-arch", trigger: "/hex-analyze-arch", desc: "Check architecture health", source: ".claude/skills/" },
  { name: "hex-feature-dev", trigger: "/hex-feature-dev", desc: "Start feature development", source: ".claude/skills/" },
  { name: "commit", trigger: "/commit", desc: "Create a git commit", source: "built-in" },
  { name: "review-pr", trigger: "/review-pr", desc: "Review a pull request", source: "built-in" },
];

const SkillsView: Component = () => {
  return (
    <div class="flex-1 overflow-auto p-6">
      {/* Header */}
      <div class="flex items-center justify-between mb-6">
        <div>
          <h2 class="text-xl font-bold text-gray-100">Skills</h2>
          <p class="mt-1 text-sm text-gray-400">
            Available slash commands and skills for Claude Code.
          </p>
        </div>
      </div>

      {/* Skill cards */}
      <div class="space-y-2">
        <For each={SKILLS}>
          {(skill) => (
            <div
              class="flex items-center gap-4 rounded-lg border border-gray-700/50 px-4 py-3"
              style={{ "background-color": "#111827" }}
            >
              {/* Name */}
              <span class="text-sm font-bold text-gray-200 min-w-[150px]">
                {skill.name}
              </span>
              {/* Trigger */}
              <span
                class="text-xs min-w-[160px]"
                style={{ "font-family": "'JetBrains Mono', monospace", color: "#22d3ee" }}
              >
                {skill.trigger}
              </span>
              {/* Description */}
              <span class="text-sm text-gray-400 flex-1">
                {skill.desc}
              </span>
              {/* Source badge */}
              <span
                class="shrink-0 rounded-full px-2.5 py-0.5 text-xs font-medium"
                classList={{
                  "bg-cyan-900/30 text-cyan-400": skill.source !== "built-in",
                  "bg-gray-800 text-gray-500": skill.source === "built-in",
                }}
              >
                {skill.source === "built-in" ? "built-in" : "custom"}
              </span>
            </div>
          )}
        </For>
      </div>
    </div>
  );
};

export default SkillsView;
