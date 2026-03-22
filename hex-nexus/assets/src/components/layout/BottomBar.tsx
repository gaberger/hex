/**
 * BottomBar.tsx — Chat input bar matching Pencil design spec.
 *
 * Compact single-line input with mode pill, connection status,
 * and inline send button. Theme-aware via CSS variables.
 *
 * Enter = send, Shift+Enter = newline, Tab = toggle mode (when empty).
 */
import { Component, createSignal, createMemo, Show, For } from 'solid-js';
import { searchCommands, type Command } from '../../stores/commands';
import { mode, toggleMode } from '../../stores/mode';
import { sendMessage, isStreaming, chatConnected } from '../../stores/chat';

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
    const max = 22 * 6;
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
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      handleSubmit();
    }
    if (e.key === 'Escape') {
      setShowHints(false);
      textareaRef?.blur();
    }
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
    <div
      class="relative flex flex-col"
      style={{
        background: 'var(--bg-surface)',
        "border-top": '1px solid var(--border)',
        padding: '8px 16px 12px 16px',
        gap: '6px',
      }}
    >
      {/* Slash command hints */}
      <Show when={showHints() && slashMatches().length > 0}>
        <div
          class="absolute bottom-full left-0 right-0 py-1 shadow-xl z-10 backdrop-blur-sm"
          style={{ background: 'var(--bg-surface)', "border-top": '1px solid var(--border)' }}
        >
          <For each={slashMatches()}>
            {(cmd) => (
              <button
                class="flex w-full items-center gap-3 px-5 py-2 text-left text-xs transition-colors"
                style={{ color: 'var(--text-muted)' }}
                onClick={() => selectHint(cmd)}
                onMouseOver={(e) => e.currentTarget.style.background = 'var(--bg-elevated)'}
                onMouseOut={(e) => e.currentTarget.style.background = 'transparent'}
              >
                <span
                  class="shrink-0 rounded px-1.5 py-0.5 text-[9px] uppercase"
                  style={{ background: 'var(--bg-elevated)', color: 'var(--accent)' }}
                >
                  {cmd.category}
                </span>
                <span style={{ color: 'var(--text-body)' }}>{cmd.label}</span>
                <Show when={cmd.shortcut}>
                  <kbd
                    class="ml-auto rounded px-1.5 py-0.5 text-[9px]"
                    style={{ border: '1px solid var(--border)', background: 'var(--bg-elevated)', color: 'var(--text-faint)' }}
                  >{cmd.shortcut}</kbd>
                </Show>
              </button>
            )}
          </For>
        </div>
      </Show>

      {/* Status row — mode pill + connection (Pencil: gap 8) */}
      <div class="flex items-center" style={{ gap: '8px' }}>
        <button
          class={`flex items-center gap-1.5 rounded-md px-2 py-0.5 text-[10px] font-semibold select-none transition-colors ${mode() === 'plan' ? 'bg-blue-900/30 text-blue-400' : 'bg-green-900/15 text-green-400'}`}
          onClick={toggleMode}
          title="Toggle Plan/Build mode (Tab)"
        >
          <span
            class={`h-1.5 w-1.5 rounded-full ${mode() === 'plan' ? 'bg-blue-400' : 'bg-green-400'}`}
          />
          {mode() === 'plan' ? 'Plan' : 'Build'}
        </button>
        <span class={`h-1.5 w-1.5 rounded-full ${chatConnected() ? 'bg-green-400' : 'bg-red-500'}`} />
        <span class="text-[10px]" style={{ color: 'var(--text-dim)' }}>
          {chatConnected() ? 'connected' : 'disconnected'}
        </span>
        <Show when={isStreaming()}>
          <span class="h-1.5 w-1.5 rounded-full animate-pulse ml-2" style={{ background: 'var(--accent)' }} />
          <span class="text-[10px]" style={{ color: 'var(--accent)' }}>streaming...</span>
        </Show>
      </div>

      {/* Input row (Pencil: rounded 10, padding [12,16], gap 12) */}
      <div
        class="flex items-center transition-colors"
        style={{
          background: 'var(--bg-base)',
          "border-radius": '10px',
          border: focused()
            ? `1px solid ${mode() === 'build' ? 'rgba(22,83,37,0.5)' : 'var(--ring-active)'}`
            : '1px solid var(--border)',
          padding: '12px 16px',
          gap: '12px',
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
          placeholder={
            isStreaming()
              ? 'Waiting for response...'
              : mode() === 'plan'
                ? 'Ask a question, discuss architecture, plan changes...'
                : 'Describe what to build, fix, or change...'
          }
          rows={1}
          class="flex-1 resize-none bg-transparent text-[14px] outline-none disabled:opacity-40 disabled:cursor-not-allowed"
          style={{
            color: 'var(--text-body)',
            "line-height": '22px',
            "min-height": '22px',
            "max-height": '132px',
          }}
        />
        <button
          onClick={handleSubmit}
          disabled={isStreaming() || !value().trim()}
          class={`flex shrink-0 items-center justify-center rounded-lg h-8 w-8 transition-colors disabled:opacity-20 disabled:cursor-not-allowed ${value().trim() ? 'bg-green-600' : ''}`}
          style={{
            ...(!value().trim() ? { background: 'var(--bg-elevated)' } : {}),
          }}
          title="Send (Enter)"
        >
          {/* Lucide send icon — 16x16: white on green when has text, muted when empty */}
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke={value().trim() ? '#ffffff' : 'var(--text-faint)'} stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M22 2 11 13" />
            <path d="M22 2 15 22 11 13 2 9 22 2" />
          </svg>
        </button>
      </div>
    </div>
  );
};

export default BottomBar;
