/**
 * MarkdownEditor.tsx — Reusable split-pane markdown editor with live preview.
 *
 * Used for ADRs, Skills, CLAUDE.md, and any markdown file.
 * Modes: view (read-only rendered), edit (split editor + preview), raw (raw markdown)
 */
import { Component, Show, createSignal, createEffect } from 'solid-js';
import MarkdownContent from '../chat/MarkdownContent';

export type EditorMode = 'view' | 'edit' | 'raw';

export interface MarkdownEditorProps {
  /** The markdown content */
  content: string;
  /** File path displayed in header */
  filePath?: string;
  /** Title displayed in header */
  title?: string;
  /** Initial mode */
  initialMode?: EditorMode;
  /** Whether the file is editable (shows edit/save buttons) */
  editable?: boolean;
  /** Called when content is saved */
  onSave?: (content: string) => void;
  /** Metadata to show in the header bar (e.g. status, date) */
  metadata?: Array<{ label: string; value: string; color?: string }>;
}

const MarkdownEditor: Component<MarkdownEditorProps> = (props) => {
  const [mode, setMode] = createSignal<EditorMode>(props.initialMode ?? 'view');
  const [editContent, setEditContent] = createSignal(props.content);
  const [dirty, setDirty] = createSignal(false);

  // Sync when props.content changes
  createEffect(() => {
    setEditContent(props.content);
    setDirty(false);
  });

  function handleInput(e: InputEvent & { currentTarget: HTMLTextAreaElement }) {
    setEditContent(e.currentTarget.value);
    setDirty(true);
  }

  function handleSave() {
    if (props.onSave) {
      props.onSave(editContent());
      setDirty(false);
    }
  }

  function handleCancel() {
    setEditContent(props.content);
    setDirty(false);
    setMode('view');
  }

  return (
    <div class="flex h-full flex-col bg-gray-950">
      {/* Toolbar */}
      <div class="flex items-center justify-between border-b border-gray-800 px-5 py-3">
        <div class="flex items-center gap-3">
          <Show when={props.title}>
            <svg class="h-5 w-5 text-orange-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
              <polyline points="14 2 14 8 20 8" />
              <line x1="16" y1="13" x2="8" y2="13" />
              <line x1="16" y1="17" x2="8" y2="17" />
              <polyline points="10 9 9 9 8 9" />
            </svg>
            <span class="text-base font-bold text-gray-100">{props.title}</span>
          </Show>
          <Show when={props.filePath}>
            <span class="text-xs text-gray-600" style={{ "font-family": "'JetBrains Mono', monospace" }}>
              {props.filePath}
            </span>
          </Show>
          <Show when={dirty()}>
            <span class="rounded bg-yellow-900/30 px-2 py-0.5 text-[10px] font-semibold text-yellow-400">
              unsaved
            </span>
          </Show>
        </div>

        {/* Mode buttons */}
        <div class="flex items-center gap-2">
          <Show when={props.editable !== false}>
            <div class="flex rounded-lg border border-gray-700 overflow-hidden">
              <button
                class="px-3 py-1.5 text-xs font-medium transition-colors"
                classList={{
                  "bg-gray-700 text-gray-100": mode() === "view",
                  "text-gray-500 hover:text-gray-300": mode() !== "view",
                }}
                onClick={() => setMode("view")}
              >
                View
              </button>
              <button
                class="px-3 py-1.5 text-xs font-medium transition-colors"
                classList={{
                  "bg-gray-700 text-gray-100": mode() === "edit",
                  "text-gray-500 hover:text-gray-300": mode() !== "edit",
                }}
                onClick={() => setMode("edit")}
              >
                Edit
              </button>
              <button
                class="px-3 py-1.5 text-xs font-medium transition-colors"
                classList={{
                  "bg-gray-700 text-gray-100": mode() === "raw",
                  "text-gray-500 hover:text-gray-300": mode() !== "raw",
                }}
                onClick={() => setMode("raw")}
              >
                Raw
              </button>
            </div>
          </Show>
          <Show when={dirty() && props.onSave}>
            <button
              class="rounded-lg bg-cyan-600 px-4 py-1.5 text-xs font-medium text-white hover:bg-cyan-500 transition-colors"
              onClick={handleSave}
            >
              Save
            </button>
            <button
              class="rounded-lg border border-gray-700 px-3 py-1.5 text-xs text-gray-400 hover:text-gray-300 transition-colors"
              onClick={handleCancel}
            >
              Cancel
            </button>
          </Show>
        </div>
      </div>

      {/* Metadata bar */}
      <Show when={props.metadata && props.metadata.length > 0}>
        <div class="flex items-center gap-5 border-b border-gray-800 bg-gray-900/50 px-5 py-2.5">
          {props.metadata!.map((m) => (
            <div class="flex items-center gap-2 text-xs">
              <span class="text-gray-500">{m.label}</span>
              <span
                class="rounded px-2 py-0.5 font-medium"
                style={{
                  color: m.color || '#d1d5db',
                  "background-color": m.color ? m.color + '20' : '#1f2937',
                }}
              >
                {m.value}
              </span>
            </div>
          ))}
        </div>
      </Show>

      {/* Content area */}
      <div class="flex-1 overflow-hidden">
        {/* VIEW MODE — rendered markdown */}
        <Show when={mode() === "view"}>
          <div class="h-full overflow-auto px-6 py-5">
            <MarkdownContent content={editContent()} />
          </div>
        </Show>

        {/* EDIT MODE — split pane */}
        <Show when={mode() === "edit"}>
          <div class="flex h-full">
            {/* Editor */}
            <div class="flex-1 border-r border-gray-800">
              <div class="px-4 py-2 text-[10px] uppercase tracking-wider text-gray-600 border-b border-gray-800">
                Editor
              </div>
              <textarea
                class="h-full w-full resize-none bg-gray-950 p-4 text-sm text-gray-300 outline-none"
                style={{
                  "font-family": "'JetBrains Mono', monospace",
                  "line-height": "1.7",
                  "tab-size": "2",
                }}
                value={editContent()}
                onInput={handleInput}
                spellcheck={false}
              />
            </div>
            {/* Preview */}
            <div class="flex-1 overflow-auto">
              <div class="px-4 py-2 text-[10px] uppercase tracking-wider text-gray-600 border-b border-gray-800">
                Preview
              </div>
              <div class="px-6 py-4">
                <MarkdownContent content={editContent()} />
              </div>
            </div>
          </div>
        </Show>

        {/* RAW MODE — read-only monospace */}
        <Show when={mode() === "raw"}>
          <div class="h-full overflow-auto">
            <pre
              class="p-5 text-sm text-gray-400 leading-relaxed"
              style={{ "font-family": "'JetBrains Mono', monospace" }}
            >
              {editContent()}
            </pre>
          </div>
        </Show>
      </div>
    </div>
  );
};

export default MarkdownEditor;
