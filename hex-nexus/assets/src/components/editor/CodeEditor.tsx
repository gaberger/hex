/**
 * CodeEditor.tsx — Reusable code/text editor with line numbers and syntax hints.
 *
 * Used for YAML, JSON, TOML, and any non-markdown file editing.
 * Modes: view (read-only formatted), edit (editable with line numbers)
 */
import { Component, Show, createSignal, createEffect, createMemo } from 'solid-js';

export interface CodeEditorProps {
  /** The file content */
  content: string;
  /** Language hint for display (yaml, json, toml, text) */
  language?: string;
  /** File path displayed in header */
  filePath?: string;
  /** Title displayed in header */
  title?: string;
  /** Whether the file is editable */
  editable?: boolean;
  /** Called when content is saved */
  onSave?: (content: string) => void;
  /** Called when editor is closed/cancelled */
  onCancel?: () => void;
  /** Compact mode — no header, just the editor area */
  compact?: boolean;
  /** Minimum height for the textarea */
  minHeight?: string;
}

const LANGUAGE_LABELS: Record<string, string> = {
  yaml: 'YAML',
  yml: 'YAML',
  json: 'JSON',
  toml: 'TOML',
  md: 'Markdown',
  ts: 'TypeScript',
  js: 'JavaScript',
  rs: 'Rust',
  text: 'Plain Text',
};

const CodeEditor: Component<CodeEditorProps> = (props) => {
  const [editContent, setEditContent] = createSignal(props.content);
  const [dirty, setDirty] = createSignal(false);
  const [editing, setEditing] = createSignal(props.editable === true);

  createEffect(() => {
    setEditContent(props.content);
    setDirty(false);
  });

  const lineCount = createMemo(() => editContent().split('\n').length);
  const charCount = createMemo(() => editContent().length);

  const langLabel = createMemo(() => {
    if (props.language) return LANGUAGE_LABELS[props.language] ?? props.language;
    const ext = props.filePath?.split('.').pop() ?? '';
    return LANGUAGE_LABELS[ext] ?? 'Text';
  });

  function handleInput(e: InputEvent & { currentTarget: HTMLTextAreaElement }) {
    setEditContent(e.currentTarget.value);
    setDirty(true);
  }

  function handleSave() {
    props.onSave?.(editContent());
    setDirty(false);
  }

  function handleCancel() {
    setEditContent(props.content);
    setDirty(false);
    setEditing(false);
    props.onCancel?.();
  }

  // Handle Tab key for indentation
  function handleKeyDown(e: KeyboardEvent & { currentTarget: HTMLTextAreaElement }) {
    if (e.key === 'Tab') {
      e.preventDefault();
      const ta = e.currentTarget;
      const start = ta.selectionStart;
      const end = ta.selectionEnd;
      const value = ta.value;
      const newValue = value.substring(0, start) + '  ' + value.substring(end);
      setEditContent(newValue);
      setDirty(true);
      // Restore cursor position
      requestAnimationFrame(() => {
        ta.selectionStart = ta.selectionEnd = start + 2;
      });
    }
    // Ctrl/Cmd+S to save
    if ((e.ctrlKey || e.metaKey) && e.key === 's') {
      e.preventDefault();
      if (dirty()) handleSave();
    }
  }

  return (
    <div
      class="flex flex-col overflow-hidden rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-base)]"
      style={{ "min-height": props.minHeight ?? "200px" }}
    >
      {/* Header — skip in compact mode */}
      <Show when={!props.compact}>
        <div
          class="flex items-center justify-between border-b border-[var(--border-subtle)] px-4 py-2 shrink-0"
        >
          <div class="flex items-center gap-2">
            <Show when={props.title}>
              <span class="text-[13px] font-semibold text-[var(--text-primary)]">
                {props.title}
              </span>
            </Show>
            <Show when={props.filePath}>
              <span
                class="font-mono text-[11px] text-[var(--text-faint)]"
              >
                {props.filePath}
              </span>
            </Show>
            <span
              class="rounded bg-[var(--accent-dim)] px-1.5 py-0.5 text-[10px] font-medium text-[var(--accent)]"
            >
              {langLabel()}
            </span>
            <Show when={dirty()}>
              <span
                class="rounded bg-[rgba(251,191,36,0.15)] px-1.5 py-0.5 text-[10px] font-semibold text-status-warning"
              >
                unsaved
              </span>
            </Show>
          </div>

          <div class="flex items-center gap-2">
            <Show when={props.editable !== false && !editing()}>
              <button
                class="rounded border border-[var(--border)] px-3 py-1 text-[11px] font-medium text-[var(--accent)] transition-colors"
                onClick={() => setEditing(true)}
              >
                Edit
              </button>
            </Show>
            <Show when={dirty()}>
              <button
                class="rounded bg-[var(--accent)] px-3 py-1 text-[11px] font-medium text-white transition-colors"
                onClick={handleSave}
              >
                Save
              </button>
              <button
                class="rounded border border-[var(--border)] px-3 py-1 text-[11px] text-[var(--text-muted)] transition-colors"
                onClick={handleCancel}
              >
                Cancel
              </button>
            </Show>
          </div>
        </div>
      </Show>

      {/* Editor area */}
      <div class="flex-1 overflow-auto">
        <Show
          when={editing()}
          fallback={
            <pre
              class="p-4 font-mono text-[13px] leading-relaxed whitespace-pre-wrap text-[var(--text-secondary)] [tab-size:2]"
            >
              {editContent() || '(empty)'}
            </pre>
          }
        >
          <textarea
            class="h-full w-full resize-none bg-[var(--bg-base)] p-4 font-mono text-[13px] leading-[1.7] text-[var(--text-secondary)] outline-none [tab-size:2]"
            style={{ "min-height": props.minHeight ?? "200px" }}
            value={editContent()}
            onInput={handleInput}
            onKeyDown={handleKeyDown}
            spellcheck={false}
          />
        </Show>
      </div>

      {/* Footer status bar */}
      <div
        class="flex items-center justify-between border-t border-[var(--border-subtle)] px-4 py-1.5 shrink-0 text-[11px] text-[var(--text-dim)]"
      >
        <span>{lineCount()} lines, {charCount()} chars</span>
        <span>{langLabel()}</span>
      </div>
    </div>
  );
};

export default CodeEditor;
