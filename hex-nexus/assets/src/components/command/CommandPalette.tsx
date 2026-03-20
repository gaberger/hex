/**
 * CommandPalette.tsx — Ctrl+P fuzzy search overlay.
 *
 * Three access methods per ADR-039:
 * 1. Ctrl+P keyboard shortcut (this overlay)
 * 2. Slash commands in chat input (handled by BottomBar)
 * 3. Direct keyboard shortcuts (handled by App.tsx)
 */
import { Component, For, Show, createSignal, createMemo, onMount, onCleanup } from "solid-js";
import { searchCommands, type Command, type CommandCategory } from "../../stores/commands";

const CATEGORY_COLORS: Record<CommandCategory, string> = {
  project: "text-green-400",
  agent: "text-cyan-400",
  swarm: "text-purple-400",
  inference: "text-yellow-400",
  session: "text-blue-400",
  view: "text-gray-300",
  settings: "text-orange-400",
};

export interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
}

const CommandPalette: Component<CommandPaletteProps> = (props) => {
  const [query, setQuery] = createSignal("");
  const [selectedIndex, setSelectedIndex] = createSignal(0);
  let inputRef: HTMLInputElement | undefined;

  const results = createMemo(() => searchCommands(query()));

  // Reset on open
  const prevOpen = { value: false };
  createMemo(() => {
    if (props.open && !prevOpen.value) {
      setQuery("");
      setSelectedIndex(0);
      setTimeout(() => inputRef?.focus(), 0);
    }
    prevOpen.value = props.open;
  });

  function execute(cmd: Command) {
    props.onClose();
    cmd.action();
  }

  function handleKeyDown(e: KeyboardEvent) {
    const len = results().length;
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setSelectedIndex((i) => (i + 1) % Math.max(len, 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setSelectedIndex((i) => (i - 1 + Math.max(len, 1)) % Math.max(len, 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      const cmd = results()[selectedIndex()];
      if (cmd) execute(cmd);
    } else if (e.key === "Escape") {
      props.onClose();
    }
  }

  // Clamp selection when results change
  createMemo(() => {
    const len = results().length;
    if (selectedIndex() >= len) setSelectedIndex(Math.max(0, len - 1));
  });

  return (
    <Show when={props.open}>
      {/* Backdrop */}
      <div
        class="fixed inset-0 z-50 flex items-start justify-center bg-black/50 backdrop-blur-sm pt-[15vh]"
        onClick={(e) => { if (e.target === e.currentTarget) props.onClose(); }}
      >
        {/* Palette */}
        <div class="w-full max-w-lg rounded-xl border border-gray-700 bg-gray-900 shadow-2xl overflow-hidden">
          {/* Search input */}
          <div class="flex items-center gap-3 border-b border-gray-800 px-4 py-3">
            <svg class="h-4 w-4 shrink-0 text-gray-300" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <circle cx="11" cy="11" r="8" />
              <line x1="21" y1="21" x2="16.65" y2="16.65" />
            </svg>
            <input
              ref={inputRef}
              type="text"
              placeholder="Type a command..."
              value={query()}
              onInput={(e) => {
                setQuery(e.currentTarget.value);
                setSelectedIndex(0);
              }}
              onKeyDown={handleKeyDown}
              class="flex-1 bg-transparent text-sm text-gray-100 placeholder-gray-500 outline-none"
              autofocus
            />
            <kbd class="rounded border border-gray-700 bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-300">
              ESC
            </kbd>
          </div>

          {/* Results */}
          <div class="max-h-[50vh] overflow-auto py-1">
            <Show
              when={results().length > 0}
              fallback={
                <div class="px-4 py-6 text-center text-xs text-gray-300">
                  No matching commands
                </div>
              }
            >
              <For each={results()}>
                {(cmd, i) => (
                  <button
                    class="flex w-full items-center gap-3 px-4 py-2.5 text-left text-sm transition-colors"
                    classList={{
                      "bg-gray-800/80 text-white": i() === selectedIndex(),
                      "text-gray-300 hover:bg-gray-800/40": i() !== selectedIndex(),
                    }}
                    onClick={() => execute(cmd)}
                    onMouseEnter={() => setSelectedIndex(i())}
                  >
                    {/* Category badge */}
                    <span
                      class={`shrink-0 text-[9px] font-semibold uppercase tracking-wider ${CATEGORY_COLORS[cmd.category] ?? "text-gray-300"}`}
                    >
                      {cmd.category}
                    </span>

                    {/* Label */}
                    <span class="flex-1 truncate">{cmd.label}</span>

                    {/* Shortcut */}
                    <Show when={cmd.shortcut}>
                      <kbd class="shrink-0 rounded border border-gray-700 bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-300">
                        {cmd.shortcut}
                      </kbd>
                    </Show>
                  </button>
                )}
              </For>
            </Show>
          </div>

          {/* Footer */}
          <div class="flex items-center justify-between border-t border-gray-800 px-4 py-2 text-[10px] text-gray-300">
            <span>{results().length} commands</span>
            <span>
              <kbd class="rounded border border-gray-700 px-1 py-0.5">↑↓</kbd> navigate
              <kbd class="ml-2 rounded border border-gray-700 px-1 py-0.5">↵</kbd> execute
            </span>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default CommandPalette;
