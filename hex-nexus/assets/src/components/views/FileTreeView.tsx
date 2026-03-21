import { type Component, For, Show, createSignal, createMemo } from 'solid-js';
import { MarkdownEditor } from '../editor';
import { navigate, route } from '../../stores/router';

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

async function fetchDir(path: string): Promise<TreeNode[]> {
  try {
    const res = await fetch(`/api/files?path=${encodeURIComponent(path)}&list=true`);
    if (!res.ok) return [];
    const data = await res.json();
    const files: string[] = data.files ?? data ?? [];
    return sortNodes(
      files.map((f) => ({
        name: f,
        path: path === '.' ? f : `${path}/${f}`,
        isDir: guessIsDir(f),
        expanded: false,
        loaded: false,
      }))
    );
  } catch {
    return [];
  }
}

async function fetchFileContent(path: string): Promise<string> {
  try {
    const res = await fetch(`/api/files?path=${encodeURIComponent(path)}`);
    if (!res.ok) return `// Error loading file: ${res.status}`;
    const data = await res.json();
    return data.content ?? '';
  } catch {
    return '// Failed to fetch file';
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
        style={{
          "padding-left": `${props.depth * 14 + 6}px`,
          color: isSelected() ? '#22D3EE' : '#D1D5DB',
          background: isSelected() ? 'rgba(34,211,238,0.08)' : 'transparent',
        }}
        onMouseEnter={(e) => {
          if (!isSelected()) (e.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,0.04)';
        }}
        onMouseLeave={(e) => {
          if (!isSelected()) (e.currentTarget as HTMLElement).style.background = 'transparent';
        }}
        onClick={() => {
          if (props.node.isDir) {
            props.onToggle(props.node);
          } else {
            props.onSelect(props.node);
          }
        }}
      >
        {/* Icon */}
        <span style={{ color: props.node.isDir ? '#FBBF24' : '#9CA3AF', "font-size": '13px', width: '16px', "text-align": 'center', "flex-shrink": '0' }}>
          {props.node.isDir
            ? (props.node.expanded ? '\u{1F4C2}' : '\u{1F4C1}')
            : '\u{1F4C4}'}
        </span>
        {/* Expand chevron for directories */}
        <Show when={props.node.isDir}>
          <svg
            class="h-3 w-3 flex-shrink-0 transition-transform"
            style={{ transform: props.node.expanded ? 'rotate(90deg)' : 'rotate(0deg)', color: '#6B7280' }}
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <path d="M9 18l6-6-6-6" />
          </svg>
        </Show>
        <Show when={!props.node.isDir}>
          <span style={{ width: '12px' }} />
        </Show>
        <span class="truncate" style={{ "font-family": "'JetBrains Mono', monospace" }}>
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

  const isMarkdown = createMemo(() => {
    const fp = selectedFile();
    return fp ? fp.endsWith('.md') || fp.endsWith('.mdx') : false;
  });

  const fileName = createMemo(() => {
    const fp = selectedFile();
    if (!fp) return '';
    return fp.split('/').pop() ?? fp;
  });

  // Load root on init
  (async () => {
    setTreeLoading(true);
    const nodes = await fetchDir('.');
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
            fetchDir(n.path).then((children) => {
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
    setLoading(true);
    const content = await fetchFileContent(node.path);
    setFileContent(content);
    setLoading(false);
  }

  return (
    <div class="flex flex-1 overflow-hidden" style={{ background: '#0a0e14' }}>
      {/* Left panel: directory tree */}
      <div
        class="flex flex-col border-r overflow-hidden"
        style={{
          width: '280px',
          "min-width": '220px',
          "max-width": '400px',
          "border-color": '#1F2937',
          background: '#0d1117',
        }}
      >
        {/* Tree header */}
        <div
          class="flex items-center gap-2 border-b px-3 py-2"
          style={{ "border-color": '#1F2937' }}
        >
          <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="#9CA3AF" stroke-width="2">
            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
          </svg>
          <span
            class="text-[11px] font-semibold uppercase"
            style={{ color: '#9CA3AF', "letter-spacing": '0.8px' }}
          >
            Files
          </span>
          <div class="flex-1" />
          {/* Refresh button */}
          <button
            class="rounded p-1 transition-colors"
            style={{ color: '#6B7280' }}
            onMouseEnter={(e) => ((e.currentTarget as HTMLElement).style.color = '#D1D5DB')}
            onMouseLeave={(e) => ((e.currentTarget as HTMLElement).style.color = '#6B7280')}
            onClick={async () => {
              setTreeLoading(true);
              const nodes = await fetchDir('.');
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
        <div class="flex-1 overflow-y-auto py-1" style={{ "scrollbar-width": 'thin' }}>
          <Show when={!treeLoading()} fallback={
            <div class="flex items-center justify-center py-8">
              <span class="text-[11px]" style={{ color: '#6B7280' }}>Loading...</span>
            </div>
          }>
            <Show when={tree().length > 0} fallback={
              <div class="px-3 py-4 text-center">
                <span class="text-[11px]" style={{ color: '#6B7280' }}>No files found</span>
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
      <div class="flex flex-1 flex-col overflow-hidden" style={{ background: '#0a0e14' }}>
        <Show
          when={selectedFile()}
          fallback={
            <div class="flex flex-1 items-center justify-center">
              <div class="text-center">
                <svg class="mx-auto mb-3 h-12 w-12" viewBox="0 0 24 24" fill="none" stroke="#374151" stroke-width="1">
                  <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                  <polyline points="14 2 14 8 20 8" />
                </svg>
                <p class="text-[12px]" style={{ color: '#6B7280' }}>
                  Select a file to preview
                </p>
              </div>
            </div>
          }
        >
          {/* File header bar */}
          <div
            class="flex items-center gap-2 border-b px-4 py-2"
            style={{ "border-color": '#1F2937', background: '#0d1117' }}
          >
            <span
              class="text-[11px] font-medium truncate"
              style={{ color: '#D1D5DB', "font-family": "'JetBrains Mono', monospace" }}
            >
              {selectedFile()}
            </span>
            <div class="flex-1" />
            <button
              class="rounded px-2 py-0.5 text-[10px] font-medium transition-colors"
              style={{ color: '#9CA3AF', border: '1px solid #374151' }}
              onMouseEnter={(e) => ((e.currentTarget as HTMLElement).style.background = '#1F2937')}
              onMouseLeave={(e) => ((e.currentTarget as HTMLElement).style.background = 'transparent')}
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
                <span class="text-[11px]" style={{ color: '#6B7280' }}>Loading...</span>
              </div>
            }>
              <Show
                when={isMarkdown()}
                fallback={
                  <pre
                    class="p-4 text-[12px] leading-relaxed"
                    style={{
                      color: '#D1D5DB',
                      "font-family": "'JetBrains Mono', monospace",
                      "white-space": 'pre-wrap',
                      "word-break": 'break-word',
                      "tab-size": '4',
                    }}
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
