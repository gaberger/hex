import { type Component, For, Show, createSignal, createMemo } from 'solid-js';
import { MarkdownEditor } from '../editor';
import { navigate, route } from '../../stores/router';
import { projects } from '../../stores/projects';
import { restClient } from '../../services/rest-client';

interface TreeNode {
  name: string;
  path: string;
  isDir: boolean;
  children?: TreeNode[];
  expanded?: boolean;
  loaded?: boolean;
}

/** Heuristic: treat entries with a dot-extension as files, rest as directories. */
function guessIsDir(name: string): boolean {
  // Hidden dirs like .git, .hex are still dirs
  if (name.startsWith('.') && !name.includes('.', 1)) return true;
  // If there's no extension after the first char, assume directory
  const lastDot = name.lastIndexOf('.');
  if (lastDot <= 0) return true;
  return false;
}

function sortNodes(nodes: TreeNode[]): TreeNode[] {
  return [...nodes].sort((a, b) => {
    if (a.isDir !== b.isDir) return a.isDir ? -1 : 1;
    return a.name.localeCompare(b.name);
  });
}

async function fetchDir(relativePath: string, projectId?: string): Promise<TreeNode[]> {
  try {
    let data: any;
    if (projectId) {
      // Use project-scoped browse API (resolves project path server-side)
      data = await restClient.get(
        `/api/${encodeURIComponent(projectId)}/browse?path=${encodeURIComponent(relativePath)}`
      );
      const entries: { name: string; kind: string }[] = data.entries ?? [];
      return sortNodes(
        entries.map((e) => ({
          name: e.name,
          path: relativePath === '' || relativePath === '.' ? e.name : `${relativePath}/${e.name}`,
          isDir: e.kind === 'dir',
          expanded: false,
          loaded: false,
        }))
      );
    } else {
      // Fallback: global file API (CWD-based)
      data = await restClient.get(`/api/files?path=${encodeURIComponent(relativePath)}&list=true`);
      const files: string[] = data.files ?? data ?? [];
      return sortNodes(
        files.map((f) => ({
          name: f,
          path: relativePath === '.' ? f : `${relativePath}/${f}`,
          isDir: guessIsDir(f),
          expanded: false,
          loaded: false,
        }))
      );
    }
  } catch {
    return [];
  }
}

async function fetchFileContent(path: string, projectId?: string): Promise<string> {
  try {
    if (projectId) {
      // Use project-scoped read API
      const data = await restClient.get(
        `/api/${encodeURIComponent(projectId)}/read/${encodeURIComponent(path)}`
      );
      return data.content ?? '';
    } else {
      const data = await restClient.get(`/api/files?path=${encodeURIComponent(path)}`);
      return data.content ?? '';
    }
  } catch (e: any) {
    return `// Error loading file: ${e.message}`;
  }
}

// ── Recursive tree node component ──────────────────────

const TreeNodeItem: Component<{
  node: TreeNode;
  depth: number;
  selectedPath: string | null;
  onSelect: (node: TreeNode) => void;
  onToggle: (node: TreeNode) => void;
}> = (props) => {
  const isSelected = createMemo(() => props.selectedPath === props.node.path);

  return (
    <>
      <button
        class="flex w-full items-center gap-1.5 rounded px-1.5 py-[3px] text-left text-[12px] transition-colors"
        classList={{
          "text-[var(--accent-hover)] bg-cyan-400/[0.08]": isSelected(),
          "text-[var(--text-secondary)] bg-transparent hover:bg-white/[0.04]": !isSelected(),
        }}
        style={{ "padding-left": `${props.depth * 14 + 6}px` }}
        onClick={() => {
          if (props.node.isDir) {
            props.onToggle(props.node);
          } else {
            props.onSelect(props.node);
          }
        }}
      >
        {/* Icon */}
        <span class="w-4 shrink-0 text-center text-[13px]" classList={{ "text-[var(--yellow)]": props.node.isDir, "text-[var(--text-muted)]": !props.node.isDir }}>
          {props.node.isDir
            ? (props.node.expanded ? '\u{1F4C2}' : '\u{1F4C1}')
            : '\u{1F4C4}'}
        </span>
        {/* Expand chevron for directories */}
        <Show when={props.node.isDir}>
          <svg
            class="h-3 w-3 flex-shrink-0 transition-transform text-[var(--text-faint)]"
            classList={{ "rotate-90": props.node.expanded, "rotate-0": !props.node.expanded }}
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <path d="M9 18l6-6-6-6" />
          </svg>
        </Show>
        <Show when={!props.node.isDir}>
          <span class="w-3" />
        </Show>
        <span class="truncate font-mono">
          {props.node.name}
        </span>
      </button>
      {/* Children */}
      <Show when={props.node.isDir && props.node.expanded && props.node.children}>
        <For each={props.node.children}>
          {(child) => (
            <TreeNodeItem
              node={child}
              depth={props.depth + 1}
              selectedPath={props.selectedPath}
              onSelect={props.onSelect}
              onToggle={props.onToggle}
            />
          )}
        </For>
      </Show>
    </>
  );
};

// ── Main FileTreeView ──────────────────────────────────

const FileTreeView: Component = () => {
  const [tree, setTree] = createSignal<TreeNode[]>([]);
  const [selectedFile, setSelectedFile] = createSignal<string | null>(null);
  const [fileContent, setFileContent] = createSignal('');
  const [loading, setLoading] = createSignal(false);
  const [treeLoading, setTreeLoading] = createSignal(true);
  const [treePanelOpen, setTreePanelOpen] = createSignal(true);

  // Extract projectId from route for project-scoped browse API
  const projectId = createMemo(() => {
    const r = route();
    return (r as any).projectId ?? "";
  });

  const isMarkdown = createMemo(() => {
    const fp = selectedFile();
    return fp ? fp.endsWith('.md') || fp.endsWith('.mdx') : false;
  });

  const fileName = createMemo(() => {
    const fp = selectedFile();
    if (!fp) return '';
    return fp.split('/').pop() ?? fp;
  });

  // Load root on init (scoped to project directory via browse API)
  (async () => {
    setTreeLoading(true);
    const pid = projectId();
    const nodes = await fetchDir('.', pid || undefined);
    setTree(nodes);
    setTreeLoading(false);
  })();

  async function handleToggle(node: TreeNode) {
    if (!node.isDir) return;

    // Deep-clone and update the tree
    function toggleInTree(nodes: TreeNode[]): TreeNode[] {
      return nodes.map((n) => {
        if (n.path === node.path) {
          const willExpand = !n.expanded;
          if (willExpand && !n.loaded) {
            // Lazy-load children
            fetchDir(n.path, projectId() || undefined).then((children) => {
              setTree((prev) => updateNodeInTree(prev, n.path, { children, loaded: true }));
            });
          }
          return { ...n, expanded: willExpand };
        }
        if (n.children) {
          return { ...n, children: toggleInTree(n.children) };
        }
        return n;
      });
    }

    setTree((prev) => toggleInTree(prev));
  }

  function updateNodeInTree(
    nodes: TreeNode[],
    path: string,
    updates: Partial<TreeNode>
  ): TreeNode[] {
    return nodes.map((n) => {
      if (n.path === path) return { ...n, ...updates };
      if (n.children) return { ...n, children: updateNodeInTree(n.children, path, updates) };
      return n;
    });
  }

  async function handleSelect(node: TreeNode) {
    setSelectedFile(node.path);
    if (window.innerWidth < 768) setTreePanelOpen(false);
    setLoading(true);
    const content = await fetchFileContent(node.path, projectId() || undefined);
    setFileContent(content);
    setLoading(false);
  }

  return (
    <div class="flex flex-1 overflow-hidden relative bg-[var(--bg-base)]">
      {/* Mobile tree panel toggle */}
      <button
        class="md:hidden absolute top-2 left-2 z-20 rounded-lg p-1.5 transition-colors"
        class="border border-[var(--border)] bg-[var(--bg-base)] text-[var(--text-muted)]"
        onClick={() => setTreePanelOpen((v) => !v)}
        title={treePanelOpen() ? 'Hide file tree' : 'Show file tree'}
      >
        <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <Show when={treePanelOpen()} fallback={<><line x1="3" y1="6" x2="21" y2="6" /><line x1="3" y1="12" x2="21" y2="12" /><line x1="3" y1="18" x2="21" y2="18" /></>}>
            <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
          </Show>
        </svg>
      </button>

      {/* Left panel: directory tree */}
      <div
        class="flex flex-col border-r overflow-hidden w-56 md:w-72 shrink-0 transition-all duration-200"
        classList={{
          'max-md:absolute max-md:inset-y-0 max-md:left-0 max-md:z-10 max-md:shadow-2xl': true,
          'max-md:-translate-x-full': !treePanelOpen(),
          'max-md:translate-x-0': treePanelOpen(),
        }}
        class="border-[var(--border-subtle)] bg-[var(--bg-base)]"
      >
        {/* Tree header */}
        <div
          class="flex items-center gap-2 border-b border-[var(--border-subtle)] px-3 py-2"
        >
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="var(--text-muted)" stroke-width="2">
            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
          </svg>
          <span
            class="text-[11px] font-semibold uppercase tracking-[0.8px] text-[var(--text-muted)]"
          >
            Files
          </span>
          <div class="flex-1" />
          {/* Refresh button */}
          <button
            class="rounded p-1 transition-colors text-[var(--text-faint)] hover:text-[var(--text-secondary)]"
            onClick={async () => {
              setTreeLoading(true);
              const nodes = await fetchDir('.', projectId() || undefined);
              setTree(nodes);
              setTreeLoading(false);
            }}
            title="Refresh file tree"
          >
            <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <polyline points="23 4 23 10 17 10" />
              <path d="M20.49 15a9 9 0 1 1-2.12-9.36L23 10" />
            </svg>
          </button>
        </div>

        {/* Tree body */}
        <div class="flex-1 overflow-y-auto py-1 [scrollbar-width:thin]">
          <Show when={!treeLoading()} fallback={
            <div class="flex items-center justify-center py-8">
              <span class="text-[11px] text-[var(--text-faint)]">Loading...</span>
            </div>
          }>
            <Show when={tree().length > 0} fallback={
              <div class="px-3 py-4 text-center">
                <span class="text-[11px] text-[var(--text-faint)]">No files found</span>
              </div>
            }>
              <For each={tree()}>
                {(node) => (
                  <TreeNodeItem
                    node={node}
                    depth={0}
                    selectedPath={selectedFile()}
                    onSelect={handleSelect}
                    onToggle={handleToggle}
                  />
                )}
              </For>
            </Show>
          </Show>
        </div>
      </div>

      {/* Right panel: file preview */}
      <div class="flex flex-1 flex-col overflow-hidden bg-[var(--bg-base)]">
        <Show
          when={selectedFile()}
          fallback={
            <div class="flex flex-1 items-center justify-center">
              <div class="text-center">
                <svg class="mx-auto mb-3 h-12 w-12" viewBox="0 0 24 24" fill="none" stroke="var(--border)" stroke-width="1">
                  <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                  <polyline points="14 2 14 8 20 8" />
                </svg>
                <p class="text-[12px] text-[var(--text-faint)]">
                  Select a file to preview
                </p>
              </div>
            </div>
          }
        >
          {/* File header bar */}
          <div
            class="flex items-center gap-2 border-b border-[var(--border-subtle)] bg-[var(--bg-base)] px-4 py-2"
          >
            <span
              class="font-mono text-[11px] font-medium truncate text-[var(--text-secondary)]"
            >
              {selectedFile()}
            </span>
            <div class="flex-1" />
            <button
              class="rounded border border-[var(--border)] px-2 py-0.5 text-[10px] font-medium text-[var(--text-muted)] transition-colors hover:bg-[var(--bg-elevated)]"
              onClick={() => {
                const fp = selectedFile();
                if (fp) navigate({ page: 'file-viewer', filePath: fp });
              }}
              title="Open in full viewer"
            >
              Open Full
            </button>
          </div>

          {/* Content */}
          <div class="flex-1 overflow-auto">
            <Show when={!loading()} fallback={
              <div class="flex items-center justify-center py-12">
                <span class="text-[11px] text-[var(--text-faint)]">Loading...</span>
              </div>
            }>
              <Show
                when={isMarkdown()}
                fallback={
                  <pre
                    class="p-4 font-mono text-[12px] leading-relaxed whitespace-pre-wrap break-words text-[var(--text-secondary)] [tab-size:4]"
                  >
                    {fileContent()}
                  </pre>
                }
              >
                <MarkdownEditor
                  content={fileContent()}
                  filePath={selectedFile() ?? ''}
                  title={fileName()}
                  initialMode="view"
                  editable={false}
                />
              </Show>
            </Show>
          </div>
        </Show>
      </div>
    </div>
  );
};

export default FileTreeView;
