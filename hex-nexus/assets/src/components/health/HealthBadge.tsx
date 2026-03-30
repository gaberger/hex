import { Component, Show } from "solid-js";
import { healthData, healthLoading, fetchHealth } from "../../stores/health";
import { setPanelContent } from "../../stores/context-panel";
import { navigate } from "../../stores/router";

function scoreColor(score: number): string {
  if (score >= 80) return "text-green-400";
  if (score >= 60) return "text-yellow-400";
  return "text-red-400";
}

function scoreBg(score: number): string {
  if (score >= 80) return "bg-green-900/30";
  if (score >= 60) return "bg-yellow-900/30";
  return "bg-red-900/30";
}

const HealthBadge: Component = () => {
  const handleClick = async () => {
    await fetchHealth();
    setPanelContent({ type: "health-detail" });
    navigate({ page: "project-health", projectId: "current" });
  };

  return (
    <button
      class="flex w-full items-center gap-2 rounded px-2 py-1.5 text-xs text-gray-400 hover:bg-gray-800 transition-colors"
      onClick={handleClick}
    >
      <svg
        class="h-3.5 w-3.5 text-gray-500"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
      >
        <path d="M22 12h-4l-3 9L9 3l-3 9H2" />
      </svg>
      <span>Health</span>
      <Show when={healthLoading()}>
        <span class="ml-auto text-[10px] text-cyan-400 animate-pulse">
          analyzing...
        </span>
      </Show>
      <Show when={!healthLoading() && healthData()}>
        {(() => {
          const d = healthData()!;
          const raw = d as any;
          const isStub = raw.ast_is_stub || raw.astIsStub || (d.health_score === 100 && (raw.file_count ?? 0) === 0);
          return isStub
            ? <span class="ml-auto rounded px-1.5 py-0.5 text-[10px] font-mono font-bold bg-gray-800 text-gray-500" title="Run `hex analyze .` to get real scores">--/100</span>
            : <span class={`ml-auto rounded px-1.5 py-0.5 text-[10px] font-mono font-bold ${scoreBg(d.health_score)} ${scoreColor(d.health_score)}`}>{d.health_score}/100</span>;
        })()}
      </Show>
      <Show when={!healthLoading() && !healthData()}>
        <span class="ml-auto text-[10px] text-gray-600">click to scan</span>
      </Show>
    </button>
  );
};

export default HealthBadge;
