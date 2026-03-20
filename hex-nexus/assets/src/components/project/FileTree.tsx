/**
 * FileTree.tsx — Collapsible file browser for a project.
 *
 * Fetches file listing from hex-nexus REST API:
 *   GET /api/projects/:projectId/files
 * Renders a recursive tree with expand/collapse, file type icons,
 * and change indicators when agents have modified files.
 */
import { Component, For, Show, createSignal, createResource, createMemo } from "solid-js";

interface FileNode {
  name: string;
  path: string;
  type: "file" | "directory";
  children?: FileNode[];
  modified?: boolean;
}

async function fetchFileTree(projectId: string): Promise<FileNode[]> {
  if (!projectId) return [];
  try {
    const res = await fetch(`/api/projects/${encodeURIComponent(projectId)}/files`);
    if (!res.ok) return [];
    const data = await res.json();
    return data.files ?? data.tree ?? data ?? [];
  } catch {
    return [];
  }
}

const FileTree: Component<{ projectId: string }> = (props) => {
  const [tree] = createResource(() => props.projectId, fetchFileTree);

  return (
    <div class="flex h-full flex-col overflow-auto bg-gray-950 p-3">
      <div class="mb-3 flex items-center justify-between">
        <h3 class="text-xs font-semibold uppercase tracking-wider text-gray-300">Files</h3>
        <span class="rounded bg-gray-800 px-1.5 py-0.5 text-[10px] text-gray-300">
          {props.projectId}
        </span>
      </div>

      <Show
        when={!tree.loading}
        fallback={<p class="text-xs text-gray-300">Loading...</p>}
      >
        <Show
          when={(tree()?.length ?? 0) > 0}
          fallback={<p class="text-xs text-gray-300">No files found</p>}
        >
          <div class="space-y-0.5">
            <For each={tree()}>
              {(node) => <TreeNode node={node} depth={0} />}
            </For>
          </div>
        </Show>
      </Show>
    </div>
  );
};

const TreeNode: Component<{ node: FileNode; depth: number }> = (props) => {
  const [expanded, setExpanded] = createSignal(props.depth < 1);
  const isDir = () => props.node.type === "directory";
  const indent = () => props.depth * 16;

  return (
    <div>
      <button
        class="flex w-full items-center gap-1.5 rounded px-1 py-1 text-left text-xs transition-colors hover:bg-gray-800/60"
        style={{ "padding-left": `${indent() + 4}px` }}
        onClick={() => isDir() && setExpanded(!expanded())}
      >
        {/* Icon */}
        <Show
          when={isDir()}
          fallback={
            <svg class="h-3.5 w-3.5 shrink-0 text-gray-300" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
              <polyline points="14 2 14 8 20 8" />
            </svg>
          }
        >
          <svg
            class="h-3.5 w-3.5 shrink-0 text-gray-300 transition-transform"
            classList={{ "rotate-90": expanded() }}
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
          >
            <polyline points="9 18 15 12 9 6" />
          </svg>
        </Show>

        {/* Name */}
        <span
          class="truncate"
          classList={{
            "text-gray-300 font-medium": isDir(),
            "text-gray-300": !isDir(),
            "text-cyan-400": props.node.modified ?? false,
          }}
        >
          {props.node.name}
        </span>

        {/* Modified indicator */}
        <Show when={props.node.modified}>
          <span class="ml-auto h-1.5 w-1.5 rounded-full bg-cyan-400" />
        </Show>
      </button>

      {/* Children */}
      <Show when={isDir() && expanded() && props.node.children}>
        <For each={props.node.children}>
          {(child) => <TreeNode node={child} depth={props.depth + 1} />}
        </For>
      </Show>
    </div>
  );
};

export default FileTree;
