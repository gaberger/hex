/**
 * BranchPicker.tsx — Dropdown for selecting git branches.
 *
 * Fetches from GET /api/{project_id}/git/branches.
 * Shows local branches first, then remote branches in a separate group.
 * Current branch is highlighted. On select, fires onBranchChange callback.
 */
import { Component, For, Show, createSignal, createEffect, createMemo, onCleanup } from "solid-js";
import { gitBranches, fetchGitBranches, type BranchInfo } from "../../stores/git";

interface BranchPickerProps {
  projectId: string;
  projectPath?: string;
  onBranchChange?: (branch: string) => void;
}

const BranchPicker: Component<BranchPickerProps> = (props) => {
  const [open, setOpen] = createSignal(false);
  const [filter, setFilter] = createSignal("");

  // Fetch branches on mount and when projectId changes
  createEffect(() => {
    if (props.projectId) {
      fetchGitBranches(props.projectId, props.projectPath);
    }
  });

  // Close on outside click
  let containerRef: HTMLDivElement | undefined;
  function handleClickOutside(e: MouseEvent) {
    if (containerRef && !containerRef.contains(e.target as Node)) {
      setOpen(false);
      setFilter("");
    }
  }
  createEffect(() => {
    if (open()) {
      document.addEventListener("mousedown", handleClickOutside);
    } else {
      document.removeEventListener("mousedown", handleClickOutside);
    }
  });
  onCleanup(() => document.removeEventListener("mousedown", handleClickOutside));

  const currentBranch = createMemo(() => {
    const head = gitBranches().find((b) => b.isHead);
    return head?.name ?? "main";
  });

  const localBranches = createMemo(() => {
    const f = filter().toLowerCase();
    return gitBranches()
      .filter((b) => !b.isRemote)
      .filter((b) => !f || b.name.toLowerCase().includes(f));
  });

  const remoteBranches = createMemo(() => {
    const f = filter().toLowerCase();
    return gitBranches()
      .filter((b) => b.isRemote)
      .filter((b) => !f || b.name.toLowerCase().includes(f));
  });

  function selectBranch(name: string) {
    setOpen(false);
    setFilter("");
    props.onBranchChange?.(name);
  }

  return (
    <div ref={containerRef} class="relative">
      {/* Trigger button */}
      <button
        class="flex items-center gap-1.5 rounded-md border px-2.5 py-1 text-[11px] font-mono transition-colors"
        style={{
          background: "var(--bg-surface)",
          "border-color": open() ? "var(--accent)" : "var(--border)",
          color: "var(--text-secondary)",
        }}
        onClick={() => setOpen(!open())}
      >
        {/* Branch icon */}
        <svg class="h-3 w-3 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
          <line x1="6" y1="3" x2="6" y2="15" />
          <circle cx="18" cy="6" r="3" />
          <circle cx="6" cy="18" r="3" />
          <path d="M18 9a9 9 0 0 1-9 9" />
        </svg>
        <span class="max-w-[140px] truncate">{currentBranch()}</span>
        {/* Chevron */}
        <svg class="h-2.5 w-2.5 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3">
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>

      {/* Dropdown */}
      <Show when={open()}>
        <div
          class="absolute left-0 top-full z-50 mt-1 w-64 rounded-lg border border-[var(--border-subtle)] bg-[var(--bg-surface)] shadow-xl overflow-hidden"
        >
          {/* Search input */}
          <div class="border-b border-[var(--border-subtle)]">
            <input
              type="text"
              placeholder="Filter branches..."
              value={filter()}
              onInput={(e) => setFilter(e.currentTarget.value)}
              class="w-full bg-transparent px-3 py-2 text-[11px] text-gray-300 placeholder-gray-600 outline-none"
              autofocus
            />
          </div>

          <div class="max-h-64 overflow-y-auto">
            {/* Local branches */}
            <Show when={localBranches().length > 0}>
              <div
                class="px-3 py-1.5 text-[9px] font-semibold uppercase tracking-wider text-[var(--text-faint)]"
              >
                Local branches
              </div>
              <For each={localBranches()}>
                {(branch) => (
                  <BranchItem
                    branch={branch}
                    isActive={branch.name === currentBranch()}
                    onSelect={() => selectBranch(branch.name)}
                  />
                )}
              </For>
            </Show>

            {/* Remote branches */}
            <Show when={remoteBranches().length > 0}>
              <div
                class="border-t border-[var(--border-subtle)] px-3 py-1.5 text-[9px] font-semibold uppercase tracking-wider text-[var(--text-faint)]"
              >
                Remote branches
              </div>
              <For each={remoteBranches()}>
                {(branch) => (
                  <BranchItem
                    branch={branch}
                    isActive={false}
                    onSelect={() => selectBranch(branch.name)}
                  />
                )}
              </For>
            </Show>

            {/* Empty state */}
            <Show when={localBranches().length === 0 && remoteBranches().length === 0}>
              <div class="px-3 py-4 text-center text-[11px] text-[var(--text-faint)]">
                {filter() ? "No matching branches" : "No branches found"}
              </div>
            </Show>
          </div>
        </div>
      </Show>
    </div>
  );
};

const BranchItem: Component<{
  branch: BranchInfo;
  isActive: boolean;
  onSelect: () => void;
}> = (props) => (
  <button
    class="flex w-full items-center gap-2 px-3 py-1.5 text-[11px] font-mono transition-colors"
    style={{
      background: props.isActive ? "var(--bg-elevated)" : "transparent",
      color: props.isActive ? "var(--accent-hover)" : "var(--text-secondary)",
    }}
    classList={{ "hover:bg-[#1E293B]/50": !props.isActive }}
    onClick={props.onSelect}
  >
    <Show when={props.isActive}>
      <svg class="h-2.5 w-2.5 shrink-0" style={{ color: "var(--accent-hover)" }} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3">
        <polyline points="20 6 9 17 4 12" />
      </svg>
    </Show>
    <Show when={!props.isActive}>
      <span class="h-2.5 w-2.5 shrink-0" />
    </Show>
    <span class="truncate">{props.branch.name}</span>
    <span class="ml-auto text-[9px] text-[var(--text-faint)]">
      {props.branch.shortSha}
    </span>
  </button>
);

export default BranchPicker;
