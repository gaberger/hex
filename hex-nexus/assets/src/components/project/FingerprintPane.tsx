/**
 * FingerprintPane.tsx — Architecture fingerprint viewer for a project.
 *
 * Displays the stored architecture fingerprint (ADR-2603301200) and provides
 * a button to regenerate it from the current project state.
 */
import { Component, Show, createResource, createSignal } from "solid-js";
import { restClient } from "../../services/rest-client";

interface ArchitectureFingerprint {
  project_id: string;
  language: string;
  framework?: string;
  architecture_style: string;
  output_type: string;
  constraints: string[];
  adr_decisions: string[];
  estimated_tokens: number;
  generated_at: string;
}

interface FingerprintPaneProps {
  projectId: string;
  projectRoot?: string;
  workplanPath?: string;
}

function styleColor(style: string): string {
  switch (style) {
    case "hexagonal": return "text-purple-400";
    case "layered": return "text-blue-400";
    default: return "text-gray-400";
  }
}

function langColor(lang: string): string {
  switch (lang) {
    case "go": return "text-cyan-400";
    case "rust": return "text-orange-400";
    case "typescript": return "text-yellow-400";
    default: return "text-gray-400";
  }
}

const FingerprintPane: Component<FingerprintPaneProps> = (props) => {
  const [regenerating, setRegenerating] = createSignal(false);
  const [regenError, setRegenError] = createSignal<string | null>(null);
  const [refetchTrigger, setRefetchTrigger] = createSignal(0);

  const [fingerprint] = createResource(
    () => [props.projectId, refetchTrigger()] as const,
    async ([projectId]) => {
      if (!projectId) return null;
      try {
        return await restClient.get<ArchitectureFingerprint>(`/api/projects/${projectId}/fingerprint`);
      } catch {
        return null;
      }
    }
  );

  async function regenerate() {
    setRegenerating(true);
    setRegenError(null);
    try {
      const body = {
        project_root: props.projectRoot ?? ".",
        workplan_path: props.workplanPath ?? "",
      };
      await restClient.post(`/api/projects/${props.projectId}/fingerprint`, body);
      setRefetchTrigger((n) => n + 1);
    } catch (e: unknown) {
      setRegenError(String(e));
    } finally {
      setRegenerating(false);
    }
  }

  return (
    <div class="flex flex-col gap-3 p-4">
      <div class="flex items-center justify-between">
        <h3 class="text-sm font-semibold text-gray-300 uppercase tracking-wider">
          Architecture Fingerprint
        </h3>
        <button
          onClick={regenerate}
          disabled={regenerating()}
          class="px-3 py-1 text-xs rounded bg-gray-700 hover:bg-gray-600 text-gray-300 disabled:opacity-50 disabled:cursor-not-allowed transition-colors"
        >
          {regenerating() ? "Regenerating…" : "Regenerate"}
        </button>
      </div>

      <Show when={regenError()}>
        <p class="text-xs text-red-400 bg-red-900/20 rounded px-2 py-1">{regenError()}</p>
      </Show>

      <Show
        when={!fingerprint.loading && fingerprint()}
        fallback={
          <Show
            when={fingerprint.loading}
            fallback={
              <div class="text-xs text-gray-500 text-center py-6">
                No fingerprint generated yet.{" "}
                <button onClick={regenerate} class="text-purple-400 underline hover:text-purple-300">
                  Generate now
                </button>
              </div>
            }
          >
            <div class="text-xs text-gray-500 animate-pulse text-center py-6">Loading…</div>
          </Show>
        }
      >
        {(fp) => (
          <div class="flex flex-col gap-2">
            {/* Core properties */}
            <div class="grid grid-cols-2 gap-x-4 gap-y-1 text-xs bg-gray-800/60 rounded p-3">
              <span class="text-gray-500">Language</span>
              <span class={`font-mono font-semibold ${langColor(fp().language)}`}>
                {fp().language}
              </span>

              <Show when={fp().framework}>
                <span class="text-gray-500">Framework</span>
                <span class="font-mono text-gray-300">{fp().framework}</span>
              </Show>

              <span class="text-gray-500">Style</span>
              <span class={`font-mono font-semibold ${styleColor(fp().architecture_style)}`}>
                {fp().architecture_style}
              </span>

              <span class="text-gray-500">Output</span>
              <span class="font-mono text-gray-300">{fp().output_type}</span>

              <span class="text-gray-500">Est. tokens</span>
              <span class="font-mono text-gray-400">{fp().estimated_tokens}</span>
            </div>

            {/* Constraints */}
            <Show when={fp().constraints?.length > 0}>
              <div class="text-xs bg-gray-800/40 rounded p-3">
                <p class="text-gray-500 mb-1 uppercase tracking-wider text-[10px]">Constraints</p>
                <ul class="flex flex-col gap-0.5">
                  {fp().constraints.map((c) => (
                    <li class="text-gray-300 before:content-['•'] before:text-purple-500 before:mr-1.5">{c}</li>
                  ))}
                </ul>
              </div>
            </Show>

            {/* ADR decisions */}
            <Show when={fp().adr_decisions?.length > 0}>
              <div class="text-xs bg-gray-800/40 rounded p-3">
                <p class="text-gray-500 mb-1 uppercase tracking-wider text-[10px]">ADR Decisions</p>
                <ul class="flex flex-col gap-0.5">
                  {fp().adr_decisions.map((d) => (
                    <li class="text-gray-300 before:content-['→'] before:text-blue-500 before:mr-1.5">{d}</li>
                  ))}
                </ul>
              </div>
            </Show>

            {/* Timestamp */}
            <p class="text-[10px] text-gray-600 text-right">
              Generated {new Date(fp().generated_at).toLocaleString()}
            </p>
          </div>
        )}
      </Show>
    </div>
  );
};

export default FingerprintPane;
