import { Component, createSignal, createEffect } from 'solid-js';
import { MarkdownEditor } from '../editor';
import { addToast } from '../../stores/toast';

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

  // TODO: Fetch real CLAUDE.md content from API
  // createEffect(async () => {
  //   const res = await fetch('/api/projects/current/files?path=CLAUDE.md');
  //   if (res.ok) { const data = await res.json(); setContent(data.content); }
  // });

  return (
    <div class="flex flex-1 flex-col overflow-hidden">
      <MarkdownEditor
        content={content()}
        title="CLAUDE.md"
        filePath="CLAUDE.md"
        initialMode="edit"
        editable={true}
        onSave={(newContent) => {
          setContent(newContent);
          addToast("info", "CLAUDE.md save requires file write API — edit CLAUDE.md directly for now");
        }}
      />
    </div>
  );
};

export default ContextView;
