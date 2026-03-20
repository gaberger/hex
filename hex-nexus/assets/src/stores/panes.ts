/**
 * panes.ts — Tiling pane state management (tmux/i3-inspired).
 *
 * The center area is a recursive binary tree:
 *   PaneNode = PaneSplit (direction + ratio + two children)
 *            | PaneLeaf  (tab group with one or more tabs)
 *
 * CONSTRAINT: Max 4 leaf panes. Beyond that, new views open as tabs
 * in the active pane's tab group. This keeps panes usable on typical
 * screen sizes (each pane stays ≥300px wide).
 */
import { createSignal, type Accessor } from "solid-js";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export const MAX_PANES = 4;

export type PaneType =
  | "project-overview"
  | "chat"
  | "swarm-monitor"
  | "filetree"
  | "taskboard"
  | "agent-log"
  | "diff"
  | "fleet-view"
  | "inference";

/** A single tab within a pane leaf. */
export interface PaneTab {
  id: string;
  paneType: PaneType;
  title: string;
  props: Record<string, any>;
}

/** A leaf node contains one or more tabs. */
export interface PaneLeaf {
  kind: "leaf";
  id: string;
  tabs: PaneTab[];
  activeTabId: string;
}

export interface PaneSplit {
  kind: "split";
  id: string;
  direction: "horizontal" | "vertical";
  ratio: number; // 0–1, first child's share
  children: [PaneNode, PaneNode];
}

export type PaneNode = PaneSplit | PaneLeaf;

// ---------------------------------------------------------------------------
// Helpers: create tabs and leaves
// ---------------------------------------------------------------------------

let _nextId = 1;
function paneId(): string {
  return `pane-${_nextId++}`;
}

function makeTab(paneType: PaneType, title: string, props: Record<string, any> = {}): PaneTab {
  return { id: paneId(), paneType, title, props };
}

function makeLeaf(tab: PaneTab): PaneLeaf {
  return { kind: "leaf", id: paneId(), tabs: [tab], activeTabId: tab.id };
}

/** Convenience: get the active tab of a leaf. */
export function activeTab(leaf: PaneLeaf): PaneTab {
  return leaf.tabs.find(t => t.id === leaf.activeTabId) ?? leaf.tabs[0];
}

// ---------------------------------------------------------------------------
// Signals
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Persistence (localStorage — survives page refresh)
// ---------------------------------------------------------------------------

const STORAGE_KEY = "hex_pane_layout";

function saveLayout(tree: PaneNode, activeId: string) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ tree, activeId, savedAt: Date.now() }));
  } catch { /* quota exceeded — ignore */ }
}

function loadLayout(): { tree: PaneNode; activeId: string } | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (!raw) return null;
    const data = JSON.parse(raw);
    // Validate basic structure
    if (data?.tree?.kind && data?.activeId) {
      // Bump ID counter past any existing IDs to avoid collisions
      const maxId = findMaxId(data.tree);
      if (maxId >= _nextId) _nextId = maxId + 1;
      return { tree: data.tree, activeId: data.activeId };
    }
  } catch { /* corrupted — ignore */ }
  return null;
}

function findMaxId(node: PaneNode): number {
  const num = parseInt(node.id.replace("pane-", ""), 10) || 0;
  if (node.kind === "leaf") {
    const tabMax = Math.max(0, ...node.tabs.map(t => parseInt(t.id.replace("pane-", ""), 10) || 0));
    return Math.max(num, tabMax);
  }
  return Math.max(num, findMaxId(node.children[0]), findMaxId(node.children[1]));
}

// ---------------------------------------------------------------------------
// Signals (restored from localStorage if available)
// ---------------------------------------------------------------------------

const saved = loadLayout();
const defaultTab = makeTab("project-overview", "Projects");
const defaultRoot = makeLeaf(defaultTab);

const [paneTree, _setPaneTree] = createSignal<PaneNode>(saved?.tree ?? defaultRoot);
const [activePaneId, setActivePaneId] = createSignal<string>(saved?.activeId ?? defaultRoot.id);
const [maximizedPaneId, setMaximizedPaneId] = createSignal<string | null>(null);

/** Wrapped setter that auto-persists. */
function setPaneTree(tree: PaneNode) {
  _setPaneTree(tree);
  saveLayout(tree, activePaneId());
}

export { paneTree, activePaneId, maximizedPaneId };

// ---------------------------------------------------------------------------
// Tree helpers
// ---------------------------------------------------------------------------

function findNode(root: PaneNode, id: string): PaneNode | null {
  if (root.id === id) return root;
  if (root.kind === "split") {
    return findNode(root.children[0], id) || findNode(root.children[1], id);
  }
  return null;
}

function replaceNode(root: PaneNode, id: string, replacement: PaneNode): PaneNode {
  if (root.id === id) return replacement;
  if (root.kind === "split") {
    return {
      ...root,
      children: [
        replaceNode(root.children[0], id, replacement),
        replaceNode(root.children[1], id, replacement),
      ],
    };
  }
  return root;
}

function removeNode(root: PaneNode, id: string): PaneNode | null {
  if (root.kind === "leaf") return root.id === id ? null : root;
  if (root.children[0].id === id) return root.children[1];
  if (root.children[1].id === id) return root.children[0];
  const left = removeNode(root.children[0], id);
  if (left !== root.children[0]) {
    return left ? { ...root, children: [left, root.children[1]] } : root.children[1];
  }
  const right = removeNode(root.children[1], id);
  if (right !== root.children[1]) {
    return right ? { ...root, children: [root.children[0], right] } : root.children[0];
  }
  return root;
}

function allLeaves(root: PaneNode): PaneLeaf[] {
  if (root.kind === "leaf") return [root];
  return [...allLeaves(root.children[0]), ...allLeaves(root.children[1])];
}

function allLeafIds(root: PaneNode): string[] {
  return allLeaves(root).map(l => l.id);
}

function leafCount(root: PaneNode): number {
  return allLeaves(root).length;
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

export function focusPane(id: string) {
  setActivePaneId(id);
}

export function toggleMaximize() {
  const current = maximizedPaneId();
  setMaximizedPaneId(current ? null : activePaneId());
}

/**
 * Smart split with 3-step progression:
 *   1 pane  → vertical split → 2 panes (top/bottom)
 *   2 panes → 2x2 grid       → 4 panes (split both halves horizontally)
 *   4 panes → add tab         → tabs in active pane
 *
 * The `direction` param is used for the 1→2 split.
 * The 2→4 split always creates a balanced grid (perpendicular to the existing split).
 */
export function splitPane(
  direction: "horizontal" | "vertical" = "vertical",
  newPaneType: PaneType = "project-overview",
  newPaneProps: Record<string, any> = {},
  newTitle: string = "New Pane",
) {
  const tree = paneTree();
  const count = leafCount(tree);

  // ── 4 panes (max) → add tab ──
  if (count >= MAX_PANES) {
    addTab(newPaneType, newTitle, newPaneProps);
    return;
  }

  // ── 2 panes → split both into 2x2 grid ──
  if (count === 2 && tree.kind === "split") {
    const perpendicular = tree.direction === "vertical" ? "horizontal" : "vertical";

    // Split each child leaf in the perpendicular direction
    const newChildren = tree.children.map((child) => {
      if (child.kind !== "leaf") return child; // shouldn't happen at count=2
      const newTab = makeTab(newPaneType, newTitle, newPaneProps);
      const newLeaf = makeLeaf(newTab);
      const split: PaneSplit = {
        kind: "split",
        id: paneId(),
        direction: perpendicular,
        ratio: 0.5,
        children: [child, newLeaf],
      };
      return split;
    }) as [PaneNode, PaneNode];

    const gridTree: PaneSplit = { ...tree, children: newChildren };
    setPaneTree(gridTree);
    // Focus the new pane in the active half
    const activeIdx = tree.children[0].id === activePaneId() ? 0 : 1;
    const newSplit = newChildren[activeIdx] as PaneSplit;
    setActivePaneId((newSplit.children[1] as PaneLeaf).id);
    setMaximizedPaneId(null);
    return;
  }

  // ── 1 pane → simple split ──
  const activeId = activePaneId();
  const target = findNode(tree, activeId);
  if (!target || target.kind !== "leaf") return;

  const newTab = makeTab(newPaneType, newTitle, newPaneProps);
  const newLeaf = makeLeaf(newTab);

  const split: PaneSplit = {
    kind: "split",
    id: paneId(),
    direction,
    ratio: 0.5,
    children: [target, newLeaf],
  };

  setPaneTree(replaceNode(tree, activeId, split));
  setActivePaneId(newLeaf.id);
  setMaximizedPaneId(null);
}

/** Add a tab to the active pane's tab group. */
export function addTab(
  paneType: PaneType,
  title: string,
  props: Record<string, any> = {},
  targetPaneId?: string,
) {
  const paneNodeId = targetPaneId ?? activePaneId();
  const tree = paneTree();
  const target = findNode(tree, paneNodeId);
  if (!target || target.kind !== "leaf") return;

  const tab = makeTab(paneType, title, props);
  const updated: PaneLeaf = {
    ...target,
    tabs: [...target.tabs, tab],
    activeTabId: tab.id,
  };
  setPaneTree(replaceNode(tree, paneNodeId, updated));
}

/** Switch to a specific tab within a pane. */
export function switchTab(paneNodeId: string, tabId: string) {
  const tree = paneTree();
  const target = findNode(tree, paneNodeId);
  if (!target || target.kind !== "leaf") return;
  if (!target.tabs.some(t => t.id === tabId)) return;

  setPaneTree(replaceNode(tree, paneNodeId, { ...target, activeTabId: tabId }));
}

/** Close a tab. If it's the last tab, close the pane. */
export function closeTab(paneNodeId: string, tabId: string) {
  const tree = paneTree();
  const target = findNode(tree, paneNodeId);
  if (!target || target.kind !== "leaf") return;

  if (target.tabs.length <= 1) {
    // Last tab — close the whole pane
    closePane(paneNodeId);
    return;
  }

  const remaining = target.tabs.filter(t => t.id !== tabId);
  const newActiveId = target.activeTabId === tabId
    ? remaining[Math.max(0, target.tabs.findIndex(t => t.id === tabId) - 1)]?.id ?? remaining[0].id
    : target.activeTabId;

  setPaneTree(replaceNode(tree, paneNodeId, {
    ...target,
    tabs: remaining,
    activeTabId: newActiveId,
  }));
}

/** Close the active pane. If it's the last pane, reset to default. */
export function closePane(id?: string) {
  const targetId = id ?? activePaneId();
  const tree = paneTree();
  const result = removeNode(tree, targetId);

  if (!result) {
    const tab = makeTab("project-overview", "Projects");
    const fresh = makeLeaf(tab);
    setPaneTree(fresh);
    setActivePaneId(fresh.id);
  } else {
    setPaneTree(result);
    if (targetId === activePaneId()) {
      const leaves = allLeafIds(result);
      if (leaves.length > 0) setActivePaneId(leaves[0]);
    }
  }
  setMaximizedPaneId(null);
}

export function resizeSplit(splitId: string, newRatio: number) {
  const clamped = Math.max(0.15, Math.min(0.85, newRatio));
  const tree = paneTree();
  const node = findNode(tree, splitId);
  if (!node || node.kind !== "split") return;
  setPaneTree(replaceNode(tree, splitId, { ...node, ratio: clamped }));
}

/**
 * Open a new view. Strategy:
 * - If only one pane with default overview → replace it
 * - If at 2+ panes → add as tab to active pane (doesn't auto-grid)
 * - Sidebar/user actions use this; keyboard splits use splitPane() for grid
 *
 * This avoids surprising grid creation when clicking sidebar items.
 * Use splitPane() explicitly for the 1→2→4 grid progression.
 */
export function openPane(
  paneType: PaneType,
  title: string,
  props: Record<string, any> = {},
) {
  const tree = paneTree();

  // Single default pane → replace it
  if (tree.kind === "leaf" && activeTab(tree).paneType === "project-overview" && tree.tabs.length === 1) {
    const tab = makeTab(paneType, title, props);
    const fresh = makeLeaf(tab);
    setPaneTree(fresh);
    setActivePaneId(fresh.id);
    return;
  }

  // Single non-default pane → split to show both
  if (tree.kind === "leaf") {
    splitPane("vertical", paneType, props, title);
    return;
  }

  // Multiple panes → add as tab to active pane
  addTab(paneType, title, props);
}

/** Replace the active tab's content (keeps the same tab slot). */
export function replaceActivePane(
  paneType: PaneType,
  title: string,
  props: Record<string, any> = {},
) {
  const paneNodeId = activePaneId();
  const tree = paneTree();
  const target = findNode(tree, paneNodeId);
  if (!target || target.kind !== "leaf") return;

  const currentTabId = target.activeTabId;
  const updatedTabs = target.tabs.map(t =>
    t.id === currentTabId ? { ...t, paneType, title, props } : t
  );

  setPaneTree(replaceNode(tree, paneNodeId, {
    ...target,
    tabs: updatedTabs,
  }));
}

export function focusNextPane() {
  const leaves = allLeafIds(paneTree());
  const idx = leaves.indexOf(activePaneId());
  if (idx >= 0 && leaves.length > 1) {
    setActivePaneId(leaves[(idx + 1) % leaves.length]);
  }
}

export function focusPrevPane() {
  const leaves = allLeafIds(paneTree());
  const idx = leaves.indexOf(activePaneId());
  if (idx >= 0 && leaves.length > 1) {
    setActivePaneId(leaves[(idx - 1 + leaves.length) % leaves.length]);
  }
}

/** Focus pane by 1-based index (Ctrl+1 through Ctrl+9). */
export function focusPaneByIndex(index: number) {
  const leaves = allLeafIds(paneTree());
  const i = index - 1; // 1-based to 0-based
  if (i >= 0 && i < leaves.length) {
    setActivePaneId(leaves[i]);
  }
}
