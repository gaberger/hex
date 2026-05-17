/**
 * CommandOutputPanel.tsx — Collapsible bottom panel showing command history.
 *
 * Displays recent commands dispatched from the dashboard with their results.
 * Replaces the need for a terminal — structured output is better than raw text.
 * Part of ADR-2603231309 (dashboard command surface).
 */
import { Component, For, Show, createMemo } from "solid-js";
import {
  commandHistory,
  panelOpen,
  setPanelOpen,
  clearHistory,
  type CommandHistoryEntry,
} from "../../stores/command-history";

const STATUS_STYLES: Record<string, string> = {
  running: "text-yellow-400 animate-pulse",
  success: "text-green-400",
  error: "text-red-400",
};

const STATUS_ICONS: Record<string, string> = {
  running: "\u25CB", // ○
  success: "\u2713", // ✓
  error: "\u2717",   // ✗
};

const CommandOutputPanel: Component = () => {
  const entries = createMemo(() => commandHistory());
  const runningCount = createMemo(() => entries().filter((e) => e.status === "running").length);
  const hasEntries = createMemo(() => entries().length > 0);

  function formatTime(iso: string): string {
    return new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  }

  return (
    <div class="fixed bottom-0 left-0 right-0 z-40 flex flex-col border-t border-gray-700 bg-gray-900/95 backdrop-blur-sm transition-all"
      classList={{ "h-8": !panelOpen(), "h-64": panelOpen() }}
    >
      {/* Header bar — always visible */}
      <button
        class="flex h-8 shrink-0 items-center justify-between px-4 text-xs text-gray-300 hover:bg-gray-800/50 transition-colors"
        onClick={() => setPanelOpen(!panelOpen())}
      >
        <div class="flex items-center gap-3">
          <span class="font-mono text-[10px] uppercase tracking-wider text-gray-500">Commands</span>
          <Show when={hasEntries()}>
            <span class="text-gray-400">{entries().length} recent</span>
          </Show>
          <Show when={runningCount() > 0}>
            <span class="text-yellow-400 animate-pulse">{runningCount()} running</span>
          </Show>
        </div>
        <div class="flex items-center gap-2">
          <Show when={hasEntries()}>
            <span
              class="text-gray-500 hover:text-gray-300 cursor-pointer"
              onClick={(e) => { e.stopPropagation(); clearHistory(); }}
            >
              Clear
            </span>
          </Show>
          <span class="text-gray-500">{panelOpen() ? "\u25BC" : "\u25B2"}</span>
        </div>
      </button>

      {/* Entries list — only when open */}
      <Show when={panelOpen()}>
        <div class="flex-1 overflow-auto px-2 py-1">
          <Show
            when={hasEntries()}
            fallback={
              <div class="flex h-full items-center justify-center text-xs text-gray-500">
                No commands executed yet. Use Ctrl+P to run commands.
              </div>
            }
          >
            <table class="w-full text-xs">
              <thead>
                <tr class="text-left text-[10px] uppercase tracking-wider text-gray-500 border-b border-gray-800">
                  <th class="w-6 py-1 px-1"></th>
                  <th class="py-1 px-2">Command</th>
                  <th class="py-1 px-2 w-20">Category</th>
                  <th class="py-1 px-2 w-20">Time</th>
                  <th class="py-1 px-2">Result</th>
                </tr>
              </thead>
              <tbody>
                <For each={entries()}>
                  {(entry: CommandHistoryEntry) => (
                    <tr class="border-b border-gray-800/50 hover:bg-gray-800/30">
                      <td class={`py-1.5 px-1 text-center ${STATUS_STYLES[entry.status]}`}>
                        {STATUS_ICONS[entry.status]}
                      </td>
                      <td class="py-1.5 px-2 text-gray-200 font-mono">{entry.label}</td>
                      <td class="py-1.5 px-2 text-gray-400">{entry.category}</td>
                      <td class="py-1.5 px-2 text-gray-500 font-mono">{formatTime(entry.startedAt)}</td>
                      <td class="py-1.5 px-2 truncate max-w-xs">
                        <Show when={entry.status === "success"}>
                          <span class="text-gray-300">{entry.result ?? "OK"}</span>
                        </Show>
                        <Show when={entry.status === "error"}>
                          <span class="text-red-400">{entry.error ?? "Failed"}</span>
                        </Show>
                        <Show when={entry.status === "running"}>
                          <span class="text-yellow-400/60">Running...</span>
                        </Show>
                      </td>
                    </tr>
                  )}
                </For>
              </tbody>
            </table>
          </Show>
        </div>
      </Show>
    </div>
  );
};

export default CommandOutputPanel;
