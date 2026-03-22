import { Component, createSignal, createEffect } from 'solid-js';
import { MarkdownEditor } from '../editor';
import { addToast } from '../../stores/toast';
import { getHexfloConn } from '../../stores/connection';
import { restClient } from '../../services/rest-client';

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
  const [loading, setLoading] = createSignal(true);

  // Fetch real CLAUDE.md content from the file read API
  createEffect(async () => {
    try {
      const data = await restClient.get('/api/files?path=CLAUDE.md');
      if (data.content) {
        setContent(data.content);
      }
    } catch {
      // API not available, keep sample content
    } finally {
      setLoading(false);
    }
  });

  return (
    <div class="flex flex-1 flex-col overflow-hidden">
      <MarkdownEditor
        content={content()}
        title="CLAUDE.md"
        filePath="CLAUDE.md"
        initialMode="edit"
        editable={true}
        onSave={async (newContent) => {
          // 1. Sync to SpacetimeDB (reactive)
          const conn = getHexfloConn();
          if (conn) {
            try {
              conn.reducers.syncConfig('claude_md', 'hex-intf', newContent, 'CLAUDE.md', new Date().toISOString());
            } catch { /* best-effort */ }
          }
          // 2. Write to file (persistent)
          try {
            await restClient.post('/api/files', { path: 'CLAUDE.md', content: newContent });
            setContent(newContent);
            addToast('success', 'CLAUDE.md saved');
          } catch (e: any) {
            addToast('error', e.message || 'Save failed — is nexus running?');
          }
        }}
      />
    </div>
  );
};

export default ContextView;
