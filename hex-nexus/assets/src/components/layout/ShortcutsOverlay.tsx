/**
 * ShortcutsOverlay.tsx — Modal overlay showing all keyboard shortcuts.
 * Triggered by Ctrl+? (Ctrl+Shift+/).
 */
import { Component, For, Show, onMount, onCleanup } from "solid-js";

interface Shortcut {
  keys: string;
  desc: string;
}

interface ShortcutSection {
  section: string;
  shortcuts: Shortcut[];
}

const SHORTCUTS: ShortcutSection[] = [
  {
    section: "Navigation",
    shortcuts: [
      { keys: "Ctrl+P", desc: "Command palette" },
      { keys: "Ctrl+N", desc: "Spawn agent" },
      { keys: "Tab", desc: "Toggle Plan/Build mode" },
      { keys: "Ctrl+Shift+?", desc: "Show this help" },
    ],
  },
  {
    section: "Project Quick Nav (when project active)",
    shortcuts: [
      { keys: "Ctrl+1", desc: "Overview" },
      { keys: "Ctrl+2", desc: "Agents" },
      { keys: "Ctrl+3", desc: "Swarms" },
      { keys: "Ctrl+4", desc: "ADRs" },
      { keys: "Ctrl+5", desc: "Chat" },
    ],
  },
  {
    section: "Editor",
    shortcuts: [
      { keys: "Enter", desc: "Send message" },
      { keys: "Shift+Enter", desc: "New line" },
      { keys: "@", desc: "File picker (planned)" },
      { keys: "/", desc: "Slash commands" },
    ],
  },
];

export interface ShortcutsOverlayProps {
  open: boolean;
  onClose: () => void;
}

const ShortcutsOverlay: Component<ShortcutsOverlayProps> = (props) => {
  function handleKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      props.onClose();
    }
  }

  onMount(() => {
    window.addEventListener("keydown", handleKeyDown);
  });
  onCleanup(() => {
    window.removeEventListener("keydown", handleKeyDown);
  });

  return (
    <Show when={props.open}>
      <div
        class="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm"
        onClick={(e) => {
          if (e.target === e.currentTarget) props.onClose();
        }}
      >
        <div class="w-full max-w-md rounded-xl border border-gray-700 bg-gray-900 shadow-2xl overflow-hidden">
          {/* Header */}
          <div class="flex items-center justify-between border-b border-gray-800 px-5 py-3">
            <h2 class="text-sm font-semibold text-gray-100">Keyboard Shortcuts</h2>
            <button
              class="rounded p-1 text-gray-400 hover:bg-gray-800 hover:text-gray-200 transition-colors"
              onClick={() => props.onClose()}
              aria-label="Close"
            >
              <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <line x1="18" y1="6" x2="6" y2="18" />
                <line x1="6" y1="6" x2="18" y2="18" />
              </svg>
            </button>
          </div>

          {/* Sections */}
          <div class="max-h-[60vh] overflow-auto px-5 py-4 space-y-5">
            <For each={SHORTCUTS}>
              {(section) => (
                <div>
                  <h3 class="text-[10px] font-semibold uppercase tracking-wider text-gray-500 mb-2">
                    {section.section}
                  </h3>
                  <div class="space-y-1.5">
                    <For each={section.shortcuts}>
                      {(shortcut) => (
                        <div class="flex items-center justify-between py-1">
                          <span class="text-sm text-gray-300">{shortcut.desc}</span>
                          <div class="flex items-center gap-1">
                            <For each={shortcut.keys.split("+")}>
                              {(key) => (
                                <kbd class="inline-block min-w-[1.5rem] rounded border border-gray-700 bg-gray-800 px-1.5 py-0.5 text-center text-[11px] font-medium text-gray-300">
                                  {key}
                                </kbd>
                              )}
                            </For>
                          </div>
                        </div>
                      )}
                    </For>
                  </div>
                </div>
              )}
            </For>
          </div>

          {/* Footer */}
          <div class="border-t border-gray-800 px-5 py-2.5 text-[10px] text-gray-500 text-center">
            Press <kbd class="rounded border border-gray-700 bg-gray-800 px-1 py-0.5 text-gray-400">ESC</kbd> to close
          </div>
        </div>
      </div>
    </Show>
  );
};

export default ShortcutsOverlay;
