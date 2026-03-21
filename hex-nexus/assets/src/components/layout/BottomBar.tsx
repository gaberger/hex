/**
 * BottomBar.tsx — OpenCode-style editor panel at the bottom.
 *
 * Full-width textarea with visible border, multi-line support,
 * auto-grow up to 8 lines, mode indicator, and slash commands.
 *
 * Enter = send, Shift+Enter = newline, Tab = toggle mode (when empty).
 */
import { Component, createSignal, createMemo, Show, For } from 'solid-js';
import { searchCommands, type Command } from '../../stores/commands';
import { mode, toggleMode } from '../../stores/mode';
import { sendMessage, isStreaming, chatConnected } from '../../stores/chat';

const MAX_ROWS = 8;
const LINE_HEIGHT = 22;

const BottomBar: Component = () => {
  let textareaRef: HTMLTextAreaElement | undefined;
  const [value, setValue] = createSignal('');
  const [showHints, setShowHints] = createSignal(false);
  const [focused, setFocused] = createSignal(false);

  const slashMatches = createMemo(() => {
    const text = value().trim();
    if (!text.startsWith('/')) return [];
    return searchCommands(text.slice(1)).slice(0, 6);
  });

  function autoGrow() {
    if (!textareaRef) return;
    textareaRef.style.height = 'auto';
    const max = LINE_HEIGHT * MAX_ROWS;
    textareaRef.style.height = Math.min(textareaRef.scrollHeight, max) + 'px';
  }

  function handleSubmit() {
    const text = value().trim();
    if (!text) return;

    if (text.startsWith('/')) {
      const matches = searchCommands(text.slice(1));
      if (matches.length > 0) {
        matches[0].action();
      } else {
        console.warn('[BottomBar] Unknown command:', text);
      }
    } else {
      sendMessage(text);
    }

    setValue('');
    setShowHints(false);
    if (textareaRef) textareaRef.style.height = 'auto';
  }

  function handleKeyDown(e: KeyboardEvent) {
    // Enter (no shift) = send
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
    if (e.key === 'Escape') {
      setShowHints(false);
      textareaRef?.blur();
    }
    // Tab: toggle mode when empty, slash-complete otherwise
    if (e.key === 'Tab') {
      if (value().trim() === '') {
        e.preventDefault();
        toggleMode();
      } else if (value().startsWith('/') && slashMatches().length > 0) {
        e.preventDefault();
        const match = slashMatches()[0];
        setValue('/' + match.label.toLowerCase().replace(/\s+/g, '-'));
      }
    }
  }

  function handleInput(e: InputEvent & { currentTarget: HTMLTextAreaElement }) {
    const text = e.currentTarget.value;
    setValue(text);
    setShowHints(text.startsWith('/') && text.length > 1);
    autoGrow();
  }

  function selectHint(cmd: Command) {
    cmd.action();
    setValue('');
    setShowHints(false);
  }

  return (
    <div class="relative border-t border-gray-800 bg-gray-900">
      {/* Slash command hints */}
      <Show when={showHints() && slashMatches().length > 0}>
        <div class="absolute bottom-full left-0 right-0 border-t border-gray-700 bg-gray-900/95 backdrop-blur-sm py-1 shadow-xl z-10">
          <For each={slashMatches()}>
            {(cmd) => (
              <button
                class="flex w-full items-center gap-3 px-5 py-2 text-left text-xs text-gray-400 hover:bg-gray-800 transition-colors"
                onClick={() => selectHint(cmd)}
              >
                <span class="shrink-0 rounded bg-gray-800 px-1.5 py-0.5 text-[9px] uppercase text-cyan-300">
                  {cmd.category}
                </span>
                <span class="text-gray-200">{cmd.label}</span>
                <Show when={cmd.shortcut}>
                  <kbd class="ml-auto rounded border border-gray-700 bg-gray-800 px-1.5 py-0.5 text-[9px] text-gray-500">{cmd.shortcut}</kbd>
                </Show>
              </button>
            )}
          </For>
        </div>
      </Show>

      {/* Status bar above input */}
      <div class="flex items-center gap-2 px-4 pt-2 pb-1">
        {/* Mode toggle */}
        <button
          class="flex items-center gap-1.5 rounded-md px-2 py-0.5 text-[10px] font-semibold transition-colors select-none"
          classList={{
            "bg-blue-900/40 text-blue-400 hover:bg-blue-900/60": mode() === "plan",
            "bg-green-900/40 text-green-400 hover:bg-green-900/60": mode() === "build",
          }}
          onClick={toggleMode}
          title="Toggle Plan/Build mode (Tab)"
        >
          <span class="h-1.5 w-1.5 rounded-full"
            classList={{
              "bg-blue-400": mode() === "plan",
              "bg-green-400": mode() === "build",
            }}
          />
          {mode() === "plan" ? "Plan" : "Build"}
        </button>

        {/* Connection status */}
        <div class="flex items-center gap-1.5">
          <span class="h-1.5 w-1.5 rounded-full" classList={{ "bg-green-500": chatConnected(), "bg-red-500": !chatConnected() }} />
          <span class="text-[10px] text-gray-600">{chatConnected() ? 'connected' : 'disconnected'}</span>
        </div>

        {/* Streaming indicator */}
        <Show when={isStreaming()}>
          <div class="flex items-center gap-1.5 text-[10px] text-cyan-500">
            <span class="h-1.5 w-1.5 rounded-full bg-cyan-500 animate-pulse" />
            streaming...
          </div>
        </Show>

        {/* Hints on right */}
        <div class="ml-auto hidden sm:flex items-center gap-2 text-[9px] text-gray-600">
          <Show when={value().startsWith('/')}>
            <span class="rounded bg-cyan-900/30 px-1.5 py-0.5 text-cyan-400">Command</span>
          </Show>
          <span>Enter send</span>
          <span>Shift+Enter newline</span>
          <kbd class="rounded border border-gray-700 bg-gray-800 px-1 py-0.5 text-gray-500">Tab</kbd>
          <span>mode</span>
        </div>
      </div>

      {/* Editor area */}
      <div class="px-4 pb-3">
        <div
          class="flex rounded-lg border bg-gray-950 transition-colors"
          classList={{
            "border-gray-700": !focused(),
            "border-blue-500/50": focused() && mode() === "plan",
            "border-green-500/50": focused() && mode() === "build",
          }}
        >
          <textarea
            ref={textareaRef}
            value={value()}
            onInput={handleInput}
            onKeyDown={handleKeyDown}
            onFocus={() => { setFocused(true); if (value().startsWith('/')) setShowHints(true); }}
            onBlur={() => { setFocused(false); setTimeout(() => setShowHints(false), 150); }}
            disabled={isStreaming()}
            placeholder={isStreaming() ? 'Waiting for response...' : mode() === "plan" ? 'Ask a question, discuss architecture, plan changes...' : 'Describe what to build, fix, or change...'}
            rows={3}
            class="flex-1 resize-none bg-transparent px-4 py-3 text-sm text-gray-100 placeholder-gray-600 outline-none disabled:opacity-40 disabled:cursor-not-allowed"
            style={{ "min-height": `${LINE_HEIGHT * 3}px`, "max-height": `${LINE_HEIGHT * MAX_ROWS}px`, "line-height": `${LINE_HEIGHT}px` }}
          />
          {/* Send button */}
          <div class="flex flex-col items-center justify-end p-2 gap-1">
            <button
              onClick={handleSubmit}
              disabled={isStreaming() || !value().trim()}
              class="flex h-8 w-8 items-center justify-center rounded-md transition-colors disabled:opacity-20 disabled:cursor-not-allowed"
              classList={{
                "bg-blue-600 hover:bg-blue-500 text-white": mode() === "plan" && !!value().trim(),
                "bg-green-600 hover:bg-green-500 text-white": mode() === "build" && !!value().trim(),
                "bg-gray-800 text-gray-600": !value().trim(),
              }}
              title="Send (Enter)"
            >
              <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
                <line x1="22" y1="2" x2="11" y2="13" />
                <polygon points="22 2 15 22 11 13 2 9 22 2" />
              </svg>
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default BottomBar;
