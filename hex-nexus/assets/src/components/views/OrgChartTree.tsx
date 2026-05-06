/**
 * OrgChartTree.tsx — SVG-based hierarchical tree visualization
 *
 * Uses D3-like tree layout algorithm to position nodes and draw connecting lines
 */

import { Component, For, createMemo, createSignal, onMount } from "solid-js";

interface AgentOrgNode {
  name: string;
  role: string;
  tier: string;
  status?: string;
  last_heartbeat?: string | null;
  active_agents?: number;
  reports_to: string | null;
  direct_reports: string[];
}

interface TreeLayoutNode {
  agent: AgentOrgNode;
  x: number;
  y: number;
  children: TreeLayoutNode[];
}

interface Props {
  nodes: AgentOrgNode[];
  selectedName: string | null;
  onSelect: (agent: AgentOrgNode) => void;
}

const OrgChartTree: Component<Props> = (props) => {
  const NODE_WIDTH = 180;
  const NODE_HEIGHT = 80;
  const HORIZONTAL_SPACING = 40;
  const VERTICAL_SPACING = 100;

  // Pan and zoom state
  const [zoom, setZoom] = createSignal(1);
  const [panX, setPanX] = createSignal(0);
  const [panY, setPanY] = createSignal(0);
  const [isDragging, setIsDragging] = createSignal(false);
  const [dragStart, setDragStart] = createSignal({ x: 0, y: 0 });

  let containerRef: HTMLDivElement | undefined;

  // Build tree structure
  const tree = createMemo(() => {
    const nodeMap = new Map<string, AgentOrgNode>();
    props.nodes.forEach(n => nodeMap.set(n.name, n));

    // Find top-level nodes (no reports_to)
    const roots = props.nodes.filter(n => !n.reports_to || n.reports_to === '');

    // Recursive function to build tree with layout
    const buildTree = (agent: AgentOrgNode, depth: number): TreeLayoutNode => {
      const children = props.nodes
        .filter(n => n.reports_to === agent.name)
        .map(child => buildTree(child, depth + 1));

      return {
        agent,
        x: 0, // Will be calculated
        y: depth * (NODE_HEIGHT + VERTICAL_SPACING),
        children
      };
    };

    return roots.map(root => buildTree(root, 0));
  });

  // Calculate horizontal positions using tree layout
  const layoutTree = createMemo(() => {
    const trees = tree();

    const calculateWidth = (node: TreeLayoutNode): number => {
      if (node.children.length === 0) return NODE_WIDTH;
      const childrenWidth = node.children.reduce((sum, child) =>
        sum + calculateWidth(child), 0);
      const spacing = Math.max(0, node.children.length - 1) * HORIZONTAL_SPACING;
      return Math.max(NODE_WIDTH, childrenWidth + spacing);
    };

    const positionNodes = (node: TreeLayoutNode, x: number): void => {
      const width = calculateWidth(node);
      node.x = x + width / 2 - NODE_WIDTH / 2;

      if (node.children.length > 0) {
        let childX = x;
        node.children.forEach(child => {
          const childWidth = calculateWidth(child);
          positionNodes(child, childX);
          childX += childWidth + HORIZONTAL_SPACING;
        });
      }
    };

    let currentX = 0;
    trees.forEach(root => {
      const width = calculateWidth(root);
      positionNodes(root, currentX);
      currentX += width + HORIZONTAL_SPACING * 2;
    });

    return trees;
  });

  // Calculate SVG dimensions
  const dimensions = createMemo(() => {
    const trees = layoutTree();
    let maxX = 0;
    let maxY = 0;

    const traverse = (node: TreeLayoutNode) => {
      maxX = Math.max(maxX, node.x + NODE_WIDTH);
      maxY = Math.max(maxY, node.y + NODE_HEIGHT);
      node.children.forEach(traverse);
    };

    trees.forEach(traverse);

    return {
      width: Math.max(1200, maxX + 40),
      height: Math.max(800, maxY + 40)
    };
  });

  // Generate connection lines
  const connections = createMemo(() => {
    const lines: Array<{ x1: number; y1: number; x2: number; y2: number }> = [];

    const traverse = (node: TreeLayoutNode) => {
      const parentCenterX = node.x + NODE_WIDTH / 2;
      const parentBottomY = node.y + NODE_HEIGHT;

      node.children.forEach(child => {
        const childCenterX = child.x + NODE_WIDTH / 2;
        const childTopY = child.y;

        // Draw line from parent bottom to child top
        lines.push({
          x1: parentCenterX,
          y1: parentBottomY,
          x2: childCenterX,
          y2: childTopY
        });

        traverse(child);
      });
    };

    layoutTree().forEach(traverse);
    return lines;
  });

  // Flatten all nodes for rendering
  const allLayoutNodes = createMemo(() => {
    const result: TreeLayoutNode[] = [];
    const traverse = (node: TreeLayoutNode) => {
      result.push(node);
      node.children.forEach(traverse);
    };
    layoutTree().forEach(traverse);
    return result;
  });

  const tierColor = (tier: string) => {
    switch (tier) {
      case "executive": return "#7c3aed"; // purple
      case "lead": return "#3b82f6"; // blue
      case "ic": return "#10b981"; // green
      default: return "#6b7280"; // gray
    }
  };

  // Mouse wheel zoom
  const handleWheel = (e: WheelEvent) => {
    e.preventDefault();
    const delta = e.deltaY > 0 ? 0.9 : 1.1;
    setZoom(z => Math.max(0.1, Math.min(5, z * delta)));
  };

  // Mouse drag pan
  const handleMouseDown = (e: MouseEvent) => {
    setIsDragging(true);
    setDragStart({ x: e.clientX - panX(), y: e.clientY - panY() });
  };

  const handleMouseMove = (e: MouseEvent) => {
    if (isDragging()) {
      setPanX(e.clientX - dragStart().x);
      setPanY(e.clientY - dragStart().y);
    }
  };

  const handleMouseUp = () => {
    setIsDragging(false);
  };

  onMount(() => {
    if (containerRef) {
      containerRef.addEventListener('wheel', handleWheel as any, { passive: false });
    }
  });

  return (
    <div class="w-full h-full flex flex-col bg-gray-950">
      {/* Zoom controls */}
      <div class="flex gap-2 p-4 border-b border-gray-800">
        <button
          onClick={() => setZoom(z => Math.min(5, z * 1.2))}
          class="px-3 py-1 bg-gray-800 hover:bg-gray-700 text-white rounded border border-gray-600"
        >
          +
        </button>
        <button
          onClick={() => setZoom(z => Math.max(0.1, z * 0.8))}
          class="px-3 py-1 bg-gray-800 hover:bg-gray-700 text-white rounded border border-gray-600"
        >
          −
        </button>
        <button
          onClick={() => { setZoom(1); setPanX(0); setPanY(0); }}
          class="px-3 py-1 bg-gray-800 hover:bg-gray-700 text-white rounded border border-gray-600"
        >
          Reset
        </button>
        <div class="px-3 py-1 text-gray-400 text-sm">
          Zoom: {Math.round(zoom() * 100)}%
        </div>
      </div>

      {/* Canvas */}
      <div
        ref={containerRef}
        class="flex-1 overflow-hidden cursor-grab active:cursor-grabbing"
        onMouseDown={handleMouseDown}
        onMouseMove={handleMouseMove}
        onMouseUp={handleMouseUp}
        onMouseLeave={handleMouseUp}
      >
        <svg
          width={dimensions().width}
          height={dimensions().height}
          class="mx-auto"
        >
          <g transform={`translate(${panX()}, ${panY()}) scale(${zoom()})`}>
            {/* Connection lines */}
            <For each={connections()}>
              {(line) => (
                <path
                  d={`M ${line.x1} ${line.y1}
                      L ${line.x1} ${line.y1 + VERTICAL_SPACING / 3}
                      L ${line.x2} ${line.y2 - VERTICAL_SPACING / 3}
                      L ${line.x2} ${line.y2}`}
                  stroke="#4b5563"
                  stroke-width="2"
                  fill="none"
                />
              )}
            </For>

            {/* Nodes */}
            <For each={allLayoutNodes()}>
              {(node) => {
                const isSelected = props.selectedName === node.agent.name;
                const color = tierColor(node.agent.tier);

                return (
                  <g
                    transform={`translate(${node.x}, ${node.y})`}
                    class="cursor-pointer"
                    onClick={() => props.onSelect(node.agent)}
                  >
                    {/* Card background */}
                    <rect
                      width={NODE_WIDTH}
                      height={NODE_HEIGHT}
                      rx="8"
                      fill="#1f2937"
                      stroke={color}
                      stroke-width={isSelected ? "4" : "2"}
                      class="transition-all hover:stroke-4"
                    />

                    {/* Name */}
                    <text
                      x={NODE_WIDTH / 2}
                      y={25}
                      text-anchor="middle"
                      fill="white"
                      font-size="14"
                      font-weight="600"
                      class="pointer-events-none"
                    >
                      {node.agent.name.length > 20
                        ? node.agent.name.substring(0, 18) + "..."
                        : node.agent.name}
                    </text>

                    {/* Role */}
                    <text
                      x={NODE_WIDTH / 2}
                      y={45}
                      text-anchor="middle"
                      fill="#9ca3af"
                      font-size="11"
                      class="pointer-events-none"
                    >
                      {node.agent.role.length > 25
                        ? node.agent.role.substring(0, 23) + "..."
                        : node.agent.role}
                    </text>

                    {/* Status indicator */}
                    {node.agent.status && (
                      <circle
                        cx={NODE_WIDTH - 15}
                        cy={15}
                        r="6"
                        fill={node.agent.status === 'online' ? '#4ade80' : '#6b7280'}
                        class="pointer-events-none"
                      />
                    )}

                    {/* Tier badge */}
                    <rect
                      x={10}
                      y={NODE_HEIGHT - 25}
                      width={NODE_WIDTH - 20}
                      height="18"
                      rx="4"
                      fill={color}
                      opacity="0.3"
                    />
                    <text
                      x={NODE_WIDTH / 2}
                      y={NODE_HEIGHT - 12}
                      text-anchor="middle"
                      fill={color}
                      font-size="10"
                      font-weight="500"
                      class="pointer-events-none"
                    >
                      {node.agent.tier.toUpperCase()}
                      {node.agent.active_agents && node.agent.active_agents > 0
                        ? ` · ${node.agent.active_agents} agent${node.agent.active_agents !== 1 ? 's' : ''}`
                        : node.children.length > 0 ? ` · ${node.children.length} reports` : ''}
                    </text>
                  </g>
                );
              }}
            </For>
          </g>
        </svg>
      </div>
    </div>
  );
};

export default OrgChartTree;
