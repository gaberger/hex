/**
 * PaneManager.tsx — Recursive tiling pane renderer.
 *
 * Renders a PaneNode tree as nested flexbox splits with draggable dividers.
 * Leaf nodes delegate to PaneLeaf for content rendering.
 * Supports maximize mode (single pane fills entire center area).
 */
import { Component, Show, createSignal, onCleanup } from "solid-js";
import type { PaneNode, PaneSplit } from "../../stores/panes";
import { paneTree, maximizedPaneId, resizeSplit } from "../../stores/panes";
import PaneLeaf from "./PaneLeaf";

// ---------------------------------------------------------------------------
// Recursive node renderer
// ---------------------------------------------------------------------------

const PaneNodeRenderer: Component<{ node: PaneNode }> = (props) => {
  return (
    <Show
      when={props.node.kind === "split"}
      fallback={<PaneLeaf node={props.node as any} />}
    >
      <SplitRenderer node={props.node as PaneSplit} />
    </Show>
  );
};

// ---------------------------------------------------------------------------
// Split renderer with draggable divider
// ---------------------------------------------------------------------------

const SplitRenderer: Component<{ node: PaneSplit }> = (props) => {
  const isHorizontal = () => props.node.direction === "horizontal";

  // Drag state for resizing
  const [dragging, setDragging] = createSignal(false);
  let containerRef: HTMLDivElement | undefined;

  function onPointerDown(e: PointerEvent) {
    e.preventDefault();
    setDragging(true);
    (e.target as HTMLElement).setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    if (!dragging() || !containerRef) return;
    const rect = containerRef.getBoundingClientRect();
    let ratio: number;
    if (isHorizontal()) {
      ratio = (e.clientX - rect.left) / rect.width;
    } else {
      ratio = (e.clientY - rect.top) / rect.height;
    }
    resizeSplit(props.node.id, ratio);
  }

  function onPointerUp() {
    setDragging(false);
  }

  return (
    <div
      ref={containerRef}
      class="flex h-full w-full"
      classList={{
        "flex-row": isHorizontal(),
        "flex-col": !isHorizontal(),
      }}
    >
      {/* First child */}
      <div
        style={{
          [isHorizontal() ? "width" : "height"]: `${props.node.ratio * 100}%`,
          "min-width": isHorizontal() ? "80px" : undefined,
          "min-height": !isHorizontal() ? "60px" : undefined,
        }}
        class="overflow-hidden"
      >
        <PaneNodeRenderer node={props.node.children[0]} />
      </div>

      {/* Divider */}
      <div
        class="shrink-0 transition-colors"
        classList={{
          "w-1 cursor-col-resize hover:bg-cyan-500/30": isHorizontal(),
          "h-1 cursor-row-resize hover:bg-cyan-500/30": !isHorizontal(),
          "bg-cyan-500/50": dragging(),
          "bg-gray-800": !dragging(),
        }}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
      />

      {/* Second child */}
      <div
        style={{
          [isHorizontal() ? "width" : "height"]: `${(1 - props.node.ratio) * 100}%`,
          "min-width": isHorizontal() ? "80px" : undefined,
          "min-height": !isHorizontal() ? "60px" : undefined,
        }}
        class="overflow-hidden"
      >
        <PaneNodeRenderer node={props.node.children[1]} />
      </div>
    </div>
  );
};

// ---------------------------------------------------------------------------
// Top-level PaneManager
// ---------------------------------------------------------------------------

/** Find a leaf node by ID in the tree. */
function findLeaf(root: PaneNode, id: string): PaneNode | null {
  if (root.id === id) return root;
  if (root.kind === "split") {
    return findLeaf(root.children[0], id) || findLeaf(root.children[1], id);
  }
  return null;
}

const PaneManager: Component = () => {
  const maximized = () => {
    const mid = maximizedPaneId();
    if (!mid) return null;
    return findLeaf(paneTree(), mid);
  };

  return (
    <div class="flex-1 overflow-hidden bg-gray-950">
      <Show
        when={!maximized()}
        fallback={
          <div class="h-full w-full">
            <PaneLeaf node={maximized()! as any} />
          </div>
        }
      >
        <PaneNodeRenderer node={paneTree()} />
      </Show>
    </div>
  );
};

export default PaneManager;
