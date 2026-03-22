/**
 * DiffViewer.tsx — Git diff viewer with staged/unstaged toggle.
 *
 * Two modes:
 *   1. Standalone (props.diff) — renders a raw diff string (original behavior)
 *   2. Connected (props.projectId) — fetches from GET /api/{project_id}/git/diff
 *
 * When connected, shows:
 *   - File-level summary (added/modified/deleted with +/- counts)
 *   - Toggle between staged and unstaged changes
 *   - Expandable hunk-level diffs with colored additions/deletions
 */
import { Component, For, Show, createSignal, createEffect, createMemo } from "solid-js";
import { gitDiff, fetchGitDiff, type DiffResult, type DiffFile } from "../../stores/git";

// ── Diff parser (shared between modes) ──────────────────

interface DiffLine {
  type: "add" | "remove" | "context" | "header";
  content: string;
  lineNum?: number;
}

function parseDiff(diff: string): DiffLine[] {
  const lines: DiffLine[] = [];
  let addNum = 0;
  let removeNum = 0;

  for (const raw of diff.split("\n")) {
    if (raw.startsWith("@@")) {
      lines.push({ type: "header", content: raw });
      const match = raw.match(/@@ -(\d+)/);
      if (match) removeNum = parseInt(match[1], 10);
      const match2 = raw.match(/\+(\d+)/);
      if (match2) addNum = parseInt(match2[1], 10);
    } else if (raw.startsWith("+")) {
      lines.push({ type: "add", content: raw.slice(1), lineNum: addNum++ });
    } else if (raw.startsWith("-")) {
      lines.push({ type: "remove", content: raw.slice(1), lineNum: removeNum++ });
    } else if (raw.startsWith("diff ") || raw.startsWith("index ") || raw.startsWith("---") || raw.startsWith("+++")) {
      lines.push({ type: "header", content: raw });
    } else {
      lines.push({ type: "context", content: raw.startsWith(" ") ? raw.slice(1) : raw, lineNum: addNum++ });
      removeNum++;
    }
  }

  return lines;
}

const LINE_STYLES = {
  add:     "bg-green-900/20 text-green-300",
  remove:  "bg-red-900/20 text-red-300",
  context: "text-gray-300",
  header:  "bg-blue-900/20 text-blue-400 font-semibold",
};

const GUTTER_STYLES = {
  add:     "text-green-600",
  remove:  "text-red-600",
  context: "text-gray-700",
  header:  "text-blue-600",
};

// ── Inline diff renderer (original simple mode) ──────────

const InlineDiff: Component<{ diff: string; filename?: string }> = (props) => {
  const lines = createMemo(() => parseDiff(props.diff));

  return (
    <div class="rounded-lg border border-gray-800 bg-gray-900/80 overflow-hidden">
      {props.filename && (
        <div class="flex items-center gap-2 border-b border-gray-800 px-3 py-1.5">
          <svg class="h-3.5 w-3.5 text-gray-300" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M12 20h9" /><path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z" />
          </svg>
          <span class="text-[11px] font-mono text-gray-300">{props.filename}</span>
          <span class="ml-auto rounded bg-gray-800 px-1.5 py-0.5 text-[9px] text-gray-300">
            diff
          </span>
        </div>
      )}
      <div class="overflow-auto">
        <pre class="text-xs leading-5 font-mono">
          <For each={lines()}>
            {(line) => (
              <div class={`flex ${LINE_STYLES[line.type]}`}>
                <span class={`w-12 shrink-0 select-none px-2 text-right ${GUTTER_STYLES[line.type]}`}>
                  {line.type === "add" ? "+" : line.type === "remove" ? "-" : line.type === "header" ? "@@" : " "}
                </span>
                <span class="flex-1 px-2 whitespace-pre">{line.content}</span>
              </div>
            )}
          </For>
        </pre>
      </div>
    </div>
  );
};

// ── File status icon ─────────────────────────────────────

const FILE_STATUS_LABELS: Record<string, { label: string; color: string }> = {
  added:    { label: "A", color: "#34D399" },
  modified: { label: "M", color: "#FBBF24" },
  deleted:  { label: "D", color: "#F87171" },
  renamed:  { label: "R", color: "#60A5FA" },
  copied:   { label: "C", color: "#A78BFA" },
  A:        { label: "A", color: "#34D399" },
  M:        { label: "M", color: "#FBBF24" },
  D:        { label: "D", color: "#F87171" },
  R:        { label: "R", color: "#60A5FA" },
};

// ── Connected DiffViewer (fetches from API) ──────────────

interface ConnectedDiffViewerProps {
  projectId: string;
  projectPath?: string;
}

const ConnectedDiffViewer: Component<ConnectedDiffViewerProps> = (props) => {
  const [staged, setStaged] = createSignal(false);
  const [expandedFiles, setExpandedFiles] = createSignal<Set<string>>(new Set());
  const [loading, setLoading] = createSignal(false);

  // Fetch diff when projectId or staged toggle changes
  createEffect(async () => {
    const pid = props.projectId;
    const isStaged = staged();
    if (!pid) return;

    setLoading(true);
    try {
      await fetchGitDiff(pid, props.projectPath, isStaged);
    } finally {
      setLoading(false);
    }
  });

  const diff = createMemo(() => gitDiff());

  const files = createMemo(() => diff()?.files ?? []);

  const totalAdded = createMemo(() => diff()?.totalAdditions ?? 0);
  const totalDeleted = createMemo(() => diff()?.totalDeletions ?? 0);

  function toggleFile(path: string) {
    const next = new Set(expandedFiles());
    if (next.has(path)) {
      next.delete(path);
    } else {
      next.add(path);
    }
    setExpandedFiles(next);
  }

  function expandAll() {
    setExpandedFiles(new Set(files().map((f) => f.path)));
  }

  function collapseAll() {
    setExpandedFiles(new Set());
  }

  return (
    <div class="flex flex-col gap-4">
      {/* Toolbar: staged/unstaged toggle + summary */}
      <div class="flex items-center gap-3">
        {/* Toggle buttons */}
        <div class="flex rounded-md border border-[var(--border)]">
          <button
            class="px-3 py-1.5 text-[11px] font-medium transition-colors"
            style={{
              background: !staged() ? "var(--bg-elevated)" : "transparent",
              color: !staged() ? "var(--accent-hover)" : "var(--text-muted)",
            }}
            onClick={() => setStaged(false)}
          >
            Unstaged
          </button>
          <button
            class="px-3 py-1.5 text-[11px] font-medium transition-colors border-l"
            style={{
              background: staged() ? "var(--bg-elevated)" : "transparent",
              color: staged() ? "var(--accent-hover)" : "var(--text-muted)",
              "border-color": "var(--border)",
            }}
            onClick={() => setStaged(true)}
          >
            Staged
          </button>
        </div>

        {/* Summary stats */}
        <Show when={!loading()}>
          <span class="text-[11px] text-[var(--text-muted)]">
            {files().length} file{files().length !== 1 ? "s" : ""} changed
          </span>
          <Show when={totalAdded() > 0}>
            <span class="text-[11px] font-mono text-status-active">
              +{totalAdded()}
            </span>
          </Show>
          <Show when={totalDeleted() > 0}>
            <span class="text-[11px] font-mono text-status-error">
              -{totalDeleted()}
            </span>
          </Show>
        </Show>

        <div class="flex-1" />

        {/* Expand/collapse buttons */}
        <Show when={files().length > 0}>
          <button
            class="text-[10px] text-[var(--text-muted)] transition-colors"
            onClick={expandAll}
          >
            Expand all
          </button>
          <button
            class="text-[10px] text-[var(--text-muted)] transition-colors"
            onClick={collapseAll}
          >
            Collapse all
          </button>
        </Show>
      </div>

      {/* Loading state */}
      <Show when={loading()}>
        <div class="flex items-center justify-center py-8">
          <div class="h-5 w-5 animate-spin rounded-full border-2 border-gray-700 border-t-cyan-400" />
          <span class="ml-2 text-[11px] text-[var(--text-muted)]">Loading diff...</span>
        </div>
      </Show>

      {/* Empty state */}
      <Show when={!loading() && files().length === 0}>
        <div class="rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-base)] p-8 text-center">
          <svg class="mx-auto mb-3 h-8 w-8 text-[var(--border)]" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
            <path d="M9 12l2 2 4-4m6 2a9 9 0 11-18 0 9 9 0 0118 0z" />
          </svg>
          <p class="text-[13px] text-[var(--text-muted)]">
            {staged() ? "No staged changes" : "No unstaged changes"}
          </p>
          <p class="mt-1 text-[11px] text-[var(--text-faint)]">
            Working tree is clean
          </p>
        </div>
      </Show>

      {/* File list with expandable diffs */}
      <Show when={!loading() && files().length > 0}>
        <div class="flex flex-col rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-base)] overflow-hidden">
          <For each={files()}>
            {(file, idx) => {
              const isExpanded = () => expandedFiles().has(file.path);
              const statusInfo = () => FILE_STATUS_LABELS[file.status] ?? { label: "?", color: "var(--text-muted)" };

              return (
                <>
                  {/* File header row */}
                  <button
                    class="flex items-center gap-2 px-3 py-2 text-left transition-colors"
                    style={{
                      background: isExpanded() ? "var(--bg-surface)" : "transparent",
                      "border-top": idx() > 0 ? "1px solid var(--border-subtle)" : "none",
                    }}
                    classList={{ "hover:bg-gray-900/50": !isExpanded() }}
                    onClick={() => toggleFile(file.path)}
                  >
                    {/* Expand chevron */}
                    <svg
                      class="h-3 w-3 shrink-0 transition-transform"
                      style={{
                        color: "var(--text-faint)",
                        transform: isExpanded() ? "rotate(90deg)" : "rotate(0deg)",
                      }}
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="3"
                    >
                      <polyline points="9 18 15 12 9 6" />
                    </svg>

                    {/* Status badge */}
                    <span
                      class="shrink-0 rounded px-1 py-0.5 text-[9px] font-bold"
                      style={{
                        color: statusInfo().color,
                        background: statusInfo().color + "15",
                      }}
                    >
                      {statusInfo().label}
                    </span>

                    {/* File path */}
                    <span class="flex-1 truncate text-[11px] font-mono text-[var(--text-secondary)]">
                      {file.path}
                    </span>

                    {/* Line counts */}
                    <Show when={file.additions > 0}>
                      <span class="text-[10px] font-mono text-status-active">
                        +{file.additions}
                      </span>
                    </Show>
                    <Show when={file.deletions > 0}>
                      <span class="text-[10px] font-mono text-status-error">
                        -{file.deletions}
                      </span>
                    </Show>
                  </button>

                  {/* Expanded diff hunks */}
                  <Show when={isExpanded() && file.patch}>
                    <div
                      class="border-t border-[var(--border-subtle)] overflow-auto"
                    >
                      <pre class="text-xs leading-5 font-mono">
                        <For each={parseDiff(file.patch)}>
                          {(line) => (
                            <div class={`flex ${LINE_STYLES[line.type]}`}>
                              <span class={`w-12 shrink-0 select-none px-2 text-right ${GUTTER_STYLES[line.type]}`}>
                                {line.type === "add" ? "+" : line.type === "remove" ? "-" : line.type === "header" ? "@@" : " "}
                              </span>
                              <span class="flex-1 px-2 whitespace-pre">{line.content}</span>
                            </div>
                          )}
                        </For>
                      </pre>
                    </div>
                  </Show>
                </>
              );
            }}
          </For>
        </div>
      </Show>

      {/* Raw diff fallback — if API returns raw text only */}
      <Show when={!loading() && files().length === 0 && diff()?.raw}>
        <InlineDiff diff={diff()!.raw} />
      </Show>
    </div>
  );
};

// ── Main export — auto-selects mode ──────────────────────

interface DiffViewerProps {
  /** Raw diff string (standalone mode) */
  diff?: string;
  /** Filename label for standalone mode */
  filename?: string;
  /** Project ID for connected mode (fetches from API) */
  projectId?: string;
  /** Project filesystem path */
  projectPath?: string;
}

const DiffViewer: Component<DiffViewerProps> = (props) => {
  return (
    <Show
      when={props.projectId}
      fallback={<InlineDiff diff={props.diff ?? ""} filename={props.filename} />}
    >
      <ConnectedDiffViewer
        projectId={props.projectId!}
        projectPath={props.projectPath}
      />
    </Show>
  );
};

export default DiffViewer;
