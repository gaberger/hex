import { Component, createSignal, createEffect, onMount } from 'solid-js';
import MarkdownContent from '../chat/MarkdownContent';

const SAMPLE_CLAUDE_MD = `# hex -- Hexagonal Architecture for LLM-Driven Development

## What This Project Is

hex is a **harness** -- a framework + CLI tool that gets **installed into target projects** for AI-driven development using hexagonal architecture (ports & adapters).

## Behavioral Rules

- Do what has been asked; nothing more, nothing less
- ALWAYS read a file before editing it
- NEVER save files to the root folder
- NEVER commit secrets, credentials, or .env files
- ALWAYS run \`bun test\` after making code changes

## Hexagonal Architecture Rules (ENFORCED)

1. **domain/** must only import from **domain/**
2. **ports/** may import from **domain/** only
3. **usecases/** may import from **domain/** and **ports/** only
4. **adapters/primary/** may import from **ports/** only
5. **adapters/secondary/** may import from **ports/** only
6. **adapters must NEVER import other adapters**
`;

const ContextView: Component = () => {
  const [content, setContent] = createSignal(SAMPLE_CLAUDE_MD);
  const [filePath] = createSignal('CLAUDE.md');
  const [dirty, setDirty] = createSignal(false);

  const handleInput = (e: Event) => {
    const target = e.target as HTMLTextAreaElement;
    setContent(target.value);
    setDirty(true);
  };

  const handleSave = () => {
    // TODO: POST to /api/projects/{projectId}/files
    setDirty(false);
  };

  return (
    <div class="flex flex-1 flex-col overflow-hidden p-6">
      {/* Header */}
      <div class="flex items-center justify-between mb-4">
        <div class="flex items-center gap-3">
          <svg class="h-4 w-4 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
            <polyline points="14 2 14 8 20 8" />
          </svg>
          <span class="text-sm text-gray-400" style={{ "font-family": "'JetBrains Mono', monospace" }}>
            {filePath()}
          </span>
          {dirty() && (
            <span class="rounded-full bg-yellow-900/30 px-2 py-0.5 text-xs text-yellow-400">unsaved</span>
          )}
        </div>
        <button
          class="rounded-lg px-4 py-2 text-sm font-medium transition-colors border"
          classList={{
            "bg-cyan-900/30 text-cyan-400 border-cyan-800 hover:bg-cyan-900/50": dirty(),
            "bg-gray-800 text-gray-500 border-gray-700 cursor-default": !dirty(),
          }}
          onClick={handleSave}
          disabled={!dirty()}
        >
          Save
        </button>
      </div>

      {/* Split pane editor + preview */}
      <div class="flex flex-1 gap-4 overflow-hidden min-h-0">
        {/* Editor */}
        <div class="flex flex-1 flex-col overflow-hidden rounded-lg border border-gray-700/50" style={{ "background-color": "#111827" }}>
          <div class="shrink-0 border-b border-gray-700/50 px-3 py-1.5 text-xs text-gray-500">
            Editor
          </div>
          <textarea
            class="flex-1 resize-none bg-transparent p-4 text-sm text-gray-300 outline-none"
            style={{ "font-family": "'JetBrains Mono', monospace", "line-height": "1.6" }}
            value={content()}
            onInput={handleInput}
            spellcheck={false}
          />
        </div>

        {/* Preview */}
        <div class="flex flex-1 flex-col overflow-hidden rounded-lg border border-gray-700/50" style={{ "background-color": "#111827" }}>
          <div class="shrink-0 border-b border-gray-700/50 px-3 py-1.5 text-xs text-gray-500">
            Preview
          </div>
          <div class="flex-1 overflow-auto p-4">
            <MarkdownContent content={content()} />
          </div>
        </div>
      </div>
    </div>
  );
};

export default ContextView;
