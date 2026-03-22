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
      class="flex flex-col overflow-hidden rounded-lg border"
      style={{
        "border-color": "var(--border-subtle)",
        background: "var(--bg-base)",
        "min-height": props.minHeight ?? "200px",
      }}
    >
      {/* Header — skip in compact mode */}
      <Show when={!props.compact}>
        <div
          class="flex items-center justify-between px-4 py-2 shrink-0"
          style={{ "border-bottom": "1px solid var(--border-subtle)" }}
        >
          <div class="flex items-center gap-2">
            <Show when={props.title}>
              <span class="text-[13px] font-semibold" style={{ color: "var(--text-primary)" }}>
                {props.title}
              </span>
            </Show>
            <Show when={props.filePath}>
              <span
                class="text-[11px]"
                style={{ color: "var(--text-faint)", "font-family": "var(--font-mono, 'JetBrains Mono', monospace)" }}
              >
                {props.filePath}
              </span>
            </Show>
            <span
              class="rounded px-1.5 py-0.5 text-[10px] font-medium"
              style={{ color: "var(--accent)", background: "var(--accent-dim)" }}
            >
              {langLabel()}
            </span>
            <Show when={dirty()}>
              <span
                class="rounded px-1.5 py-0.5 text-[10px] font-semibold"
                style={{ color: "#FBBF24", background: "rgba(251, 191, 36, 0.15)" }}
              >
                unsaved
              </span>
            </Show>
          </div>

          <div class="flex items-center gap-2">
            <Show when={props.editable !== false && !editing()}>
              <button
                class="rounded px-3 py-1 text-[11px] font-medium transition-colors"
                style={{ color: "var(--accent)", border: "1px solid var(--border)" }}
                onClick={() => setEditing(true)}
              >
                Edit
              </button>
            </Show>
            <Show when={dirty()}>
              <button
                class="rounded px-3 py-1 text-[11px] font-medium text-white transition-colors"
                style={{ background: "var(--accent)" }}
                onClick={handleSave}
              >
                Save
              </button>
              <button
                class="rounded px-3 py-1 text-[11px] transition-colors"
                style={{ color: "var(--text-muted)", border: "1px solid var(--border)" }}
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
              class="p-4 text-[13px] leading-relaxed whitespace-pre-wrap"
              style={{
                color: "var(--text-secondary)",
                "font-family": "var(--font-mono, 'JetBrains Mono', monospace)",
                "tab-size": "2",
              }}
            >
              {editContent() || '(empty)'}
            </pre>
          }
        >
          <textarea
            class="h-full w-full resize-none p-4 text-[13px] outline-none"
            style={{
              background: "var(--bg-base)",
              color: "var(--text-secondary)",
              "font-family": "var(--font-mono, 'JetBrains Mono', monospace)",
              "line-height": "1.7",
              "tab-size": "2",
              "min-height": props.minHeight ?? "200px",
            }}
            value={editContent()}
            onInput={handleInput}
            onKeyDown={handleKeyDown}
            spellcheck={false}
          />
        </Show>
      </div>

      {/* Footer status bar */}
      <div
        class="flex items-center justify-between px-4 py-1.5 shrink-0"
        style={{
          "border-top": "1px solid var(--border-subtle)",
          color: "var(--text-dim)",
          "font-size": "11px",
        }}
      >
        <span>{lineCount()} lines, {charCount()} chars</span>
        <span>{langLabel()}</span>
      </div>
    </div>
  );
};

export default CodeEditor;
