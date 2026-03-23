/**
 * EnforcementPane.tsx — Enforcement rules and mode display.
 *
 * Shows the current enforcement mode (mandatory/advisory/disabled)
 * and all configured enforcement rules with severity indicators.
 */
import { Component, For, Show, createResource } from "solid-js";
import { restClient } from "../../services/rest-client";

interface EnforcementRule {
  id: string;
  adr_ref: string;
  severity: string;
  message: string;
  enabled: boolean;
}

interface EnforcementData {
  mode: string;
  rules: EnforcementRule[];
}

function severityIcon(severity: string): { class: string; path: string } {
  switch (severity.toLowerCase()) {
    case "error":
      return {
        class: "text-red-400",
        path: "M6 18L18 6M6 6l12 12", // X mark
      };
    case "warning":
      return {
        class: "text-yellow-400",
        path: "M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126z", // triangle
      };
    default:
      return {
        class: "text-blue-400",
        path: "M11.25 11.25l.041-.02a.75.75 0 011.063.852l-.708 2.836a.75.75 0 001.063.853l.041-.021M21 12a9 9 0 11-18 0 9 9 0 0118 0zm-9-3.75h.008v.008H12V8.25z", // info circle
      };
  }
}

function modeBadgeClass(mode: string): string {
  switch (mode.toLowerCase()) {
    case "mandatory":
      return "bg-red-900/40 text-red-400 border-red-800/40";
    case "advisory":
      return "bg-yellow-900/40 text-yellow-400 border-yellow-800/40";
    case "disabled":
      return "bg-gray-800 text-gray-500 border-gray-700";
    default:
      return "bg-gray-800 text-gray-400 border-gray-700";
  }
}

const EnforcementPane: Component = () => {
  const [data, { refetch }] = createResource(async () => {
    return restClient.get<EnforcementData>("/api/hexflo/enforcement-rules");
  });

  return (
    <div class="flex flex-col gap-4 p-4">
      {/* Header */}
      <div class="flex items-center justify-between">
        <h2 class="text-sm font-semibold text-gray-200">Enforcement</h2>
        <button
          class="rounded border border-gray-700 px-3 py-1 text-xs text-gray-400 hover:border-cyan-600 hover:text-cyan-300 transition-colors disabled:opacity-50"
          onClick={() => refetch()}
          disabled={data.loading}
        >
          <Show when={data.loading} fallback="Refresh">
            <span class="animate-pulse">Loading...</span>
          </Show>
        </button>
      </div>

      {/* Loading state */}
      <Show when={data.loading && !data()}>
        <div class="flex flex-col items-center justify-center py-12 text-gray-500">
          <svg
            class="h-8 w-8 animate-spin text-cyan-400"
            viewBox="0 0 24 24"
            fill="none"
          >
            <circle
              cx="12"
              cy="12"
              r="10"
              stroke="currentColor"
              stroke-width="3"
              stroke-dasharray="31.4 31.4"
              stroke-linecap="round"
            />
          </svg>
          <span class="mt-3 text-xs">Loading enforcement rules...</span>
        </div>
      </Show>

      {/* Empty state */}
      <Show when={!data.loading && !data()}>
        <div class="flex flex-col items-center justify-center py-12 text-gray-500">
          <svg
            class="h-10 w-10 text-gray-700"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
          >
            <path d="M9 12.75L11.25 15 15 9.75m-3-7.036A11.959 11.959 0 013.598 6 11.99 11.99 0 003 9.749c0 5.592 3.824 10.29 9 11.623 5.176-1.332 9-6.03 9-11.622 0-1.31-.21-2.571-.598-3.751h-.152c-3.196 0-6.1-1.248-8.25-3.285z" />
          </svg>
          <p class="mt-3 text-xs">No enforcement rules configured</p>
        </div>
      </Show>

      {/* Rules display */}
      <Show when={data()}>
        {(d) => (
          <>
            {/* Mode banner */}
            <div class="flex items-center gap-2">
              <span class="text-xs text-gray-400">Mode:</span>
              <span
                class={`rounded border px-2 py-0.5 text-xs font-semibold ${modeBadgeClass(d().mode)}`}
              >
                {d().mode}
              </span>
            </div>

            {/* Rules list */}
            <Show
              when={d().rules && d().rules.length > 0}
              fallback={
                <p class="py-4 text-center text-xs text-gray-500">
                  No enforcement rules configured
                </p>
              }
            >
              <div class="rounded-lg border border-gray-800 bg-gray-950 divide-y divide-gray-800/50">
                <For each={d().rules}>
                  {(rule) => {
                    const icon = severityIcon(rule.severity);
                    return (
                      <div class="flex items-start gap-3 px-3 py-2.5">
                        {/* Severity icon */}
                        <svg
                          class={`mt-0.5 h-4 w-4 shrink-0 ${icon.class}`}
                          viewBox="0 0 24 24"
                          fill="none"
                          stroke="currentColor"
                          stroke-width="2"
                        >
                          <path d={icon.path} />
                        </svg>

                        {/* Rule content */}
                        <div class="flex-1 min-w-0">
                          <div class="flex items-center gap-2">
                            <span class="font-mono text-xs text-gray-300">
                              {rule.id}
                            </span>
                            <Show when={rule.adr_ref}>
                              <span class="rounded bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-400">
                                {rule.adr_ref}
                              </span>
                            </Show>
                          </div>
                          <p class="mt-0.5 text-xs text-gray-500">
                            {rule.message}
                          </p>
                        </div>

                        {/* Enabled/disabled indicator */}
                        <span
                          class="mt-0.5 shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium"
                          classList={{
                            "bg-green-900/40 text-green-400": rule.enabled,
                            "bg-gray-800 text-gray-500": !rule.enabled,
                          }}
                        >
                          {rule.enabled ? "enabled" : "disabled"}
                        </span>
                      </div>
                    );
                  }}
                </For>
              </div>
            </Show>
          </>
        )}
      </Show>
    </div>
  );
};

export default EnforcementPane;
