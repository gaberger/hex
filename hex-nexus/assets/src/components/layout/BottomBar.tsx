/**
 * BottomBar.tsx — Chat input with slash command parsing.
 *
 * Three input modes (ADR-039 Section 4):
 * 1. Plain text → sends message via WebSocket /ws/chat
 * 2. /command → dispatches to command registry
 * 3. Ctrl+P → opens CommandPalette (handled by App.tsx)
 */
import { Component, createSignal, createMemo, Show, For } from 'solid-js';
import { searchCommands, getAllCommands, type Command } from '../../stores/commands';
import { setCommandPaletteOpen } from '../../stores/ui';

/** Slash command definitions mapped from the command registry. */
function commandsAsSlash(): { slash: string; cmd: Command }[] {
  return getAllCommands().map(cmd => ({
    slash: "/" + cmd.id.replace(/\./g, " "),
    cmd,
  }));
}

const BottomBar: Component = () => {
  const [value, setValue] = createSignal('');
  const [showHints, setShowHints] = createSignal(false);

  const slashMatches = createMemo(() => {
    const text = value().trim();
    if (!text.startsWith('/')) return [];
    const query = text.slice(1); // remove leading /
    return searchCommands(query).slice(0, 6);
  });

  function handleSubmit() {
    const text = value().trim();
    if (!text) return;

    if (text.startsWith('/')) {
      // Slash command dispatch
      const query = text.slice(1);
      const matches = searchCommands(query);
      if (matches.length > 0) {
        matches[0].action();
      } else {
        console.warn('[BottomBar] Unknown command:', text);
      }
    } else {
      // Chat message — send via WebSocket
      sendChatMessage(text);
    }

    setValue('');
    setShowHints(false);
  }

  function sendChatMessage(text: string) {
    // Connect to hex-nexus /ws/chat and send
    try {
      const wsUrl = `ws://${location.hostname}:5555/ws/chat`;
      const ws = new WebSocket(wsUrl);
      ws.onopen = () => {
        ws.send(JSON.stringify({
          type: "message",
          content: text,
          role: "user",
        }));
        // WebSocket stays open for streaming response
      };
      ws.onerror = () => {
        console.error('[BottomBar] WebSocket error — is hex-nexus running?');
      };
    } catch (e) {
      console.error('[BottomBar] Failed to send:', e);
    }
  }

  function handleKeyDown(e: KeyboardEvent) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
    if (e.key === 'Escape') {
      setShowHints(false);
    }
    // Tab completion for slash commands
    if (e.key === 'Tab' && value().startsWith('/') && slashMatches().length > 0) {
      e.preventDefault();
      const match = slashMatches()[0];
      setValue('/' + match.label.toLowerCase().replace(/\s+/g, '-'));
    }
  }

  function handleInput(e: InputEvent & { currentTarget: HTMLInputElement }) {
    const text = e.currentTarget.value;
    setValue(text);
    setShowHints(text.startsWith('/') && text.length > 1);
  }

  function selectHint(cmd: Command) {
    cmd.action();
    setValue('');
    setShowHints(false);
  }

  return (
    <div class="relative">
      {/* Slash command hints */}
      <Show when={showHints() && slashMatches().length > 0}>
        <div class="absolute bottom-full left-0 right-0 border-t border-gray-800 bg-gray-900 py-1 shadow-lg">
          <For each={slashMatches()}>
            {(cmd) => (
              <button
                class="flex w-full items-center gap-3 px-4 py-2 text-left text-xs text-gray-300 hover:bg-gray-800 transition-colors"
                onClick={() => selectHint(cmd)}
              >
                <span class="shrink-0 rounded bg-gray-800 px-1.5 py-0.5 text-[9px] uppercase text-cyan-300">
                  {cmd.category}
                </span>
                <span class="text-gray-100">{cmd.label}</span>
                <Show when={cmd.shortcut}>
                  <span class="ml-auto text-gray-300">{cmd.shortcut}</span>
                </Show>
              </button>
            )}
          </For>
        </div>
      </Show>

      {/* Input bar */}
      <div class="flex items-center gap-3 border-t border-gray-800 bg-gray-900 px-4 py-2">
        <span class="text-gray-300 text-sm select-none">&gt;</span>
        <input
          type="text"
          class="flex-1 bg-transparent text-sm text-gray-100 placeholder-gray-300 outline-none"
          placeholder="Type a message or / for commands..."
          value={value()}
          onInput={handleInput}
          onKeyDown={handleKeyDown}
          onFocus={() => { if (value().startsWith('/')) setShowHints(true); }}
          onBlur={() => setTimeout(() => setShowHints(false), 150)}
        />
        <Show when={!value().startsWith('/')}>
          <span class="shrink-0 rounded bg-gray-800 px-2 py-0.5 text-[10px] font-medium text-gray-100">
            Session
          </span>
        </Show>
        <Show when={value().startsWith('/')}>
          <span class="shrink-0 rounded bg-cyan-900/40 px-2 py-0.5 text-[10px] font-medium text-cyan-300">
            Command
          </span>
        </Show>
      </div>
    </div>
  );
};

export default BottomBar;
