/**
 * TaskDAG.tsx — SVG task dependency graph with live status coloring.
 *
 * Renders tasks as nodes in a horizontal flow layout.
 * Edges show dependencies. Status colors update live via SpacetimeDB.
 * Click a task node to open the assigned agent's log.
 */
import { Component, For, createMemo } from "solid-js";
import { openPane } from "../../stores/panes";

interface TaskNode {
  id: string;
  title: string;
  status: string;
  agentId?: string;
  agentName?: string;
  dependsOn?: string[];
}

const STATUS_COLORS: Record<string, { fill: string; stroke: string; text: string }> = {
  pending:     { fill: "#1f2937", stroke: "#374151", text: "#9ca3af" },
  in_progress: { fill: "#164e63", stroke: "#06b6d4", text: "#67e8f9" },
  completed:   { fill: "#14532d", stroke: "#22c55e", text: "#86efac" },
  failed:      { fill: "#450a0a", stroke: "#ef4444", text: "#fca5a5" },
};

const NODE_W = 180;
const NODE_H = 56;
const GAP_X = 60;
const GAP_Y = 20;
const PADDING = 30;

/** Simple topological layout: group by depth (BFS from roots). */
function layoutTasks(tasks: TaskNode[]): { x: number; y: number; task: TaskNode }[] {
  const taskMap = new Map(tasks.map(t => [t.id, t]));
  const depths = new Map<string, number>();

  // BFS to compute depth
  function getDepth(id: string, visited = new Set<string>()): number {
    if (depths.has(id)) return depths.get(id)!;
    if (visited.has(id)) return 0; // cycle guard
    visited.add(id);
    const task = taskMap.get(id);
    if (!task?.dependsOn?.length) {
      depths.set(id, 0);
      return 0;
    }
    const maxParent = Math.max(...task.dependsOn.map(pid => getDepth(pid, visited)));
    const d = maxParent + 1;
    depths.set(id, d);
    return d;
  }

  tasks.forEach(t => getDepth(t.id));

  // Group by depth
  const columns = new Map<number, TaskNode[]>();
  tasks.forEach(t => {
    const d = depths.get(t.id) ?? 0;
    if (!columns.has(d)) columns.set(d, []);
    columns.get(d)!.push(t);
  });

  const result: { x: number; y: number; task: TaskNode }[] = [];
  const sortedCols = [...columns.entries()].sort((a, b) => a[0] - b[0]);

  for (const [col, colTasks] of sortedCols) {
    colTasks.forEach((task, row) => {
      result.push({
        x: PADDING + col * (NODE_W + GAP_X),
        y: PADDING + row * (NODE_H + GAP_Y),
        task,
      });
    });
  }

  return result;
}

const TaskDAG: Component<{ tasks: TaskNode[] }> = (props) => {
  const layout = createMemo(() => layoutTasks(props.tasks));

  const posMap = createMemo(() => {
    const m = new Map<string, { x: number; y: number }>();
    for (const n of layout()) m.set(n.task.id, { x: n.x, y: n.y });
    return m;
  });

  const svgWidth = createMemo(() => {
    const xs = layout().map(n => n.x);
    return xs.length ? Math.max(...xs) + NODE_W + PADDING * 2 : 400;
  });

  const svgHeight = createMemo(() => {
    const ys = layout().map(n => n.y);
    return ys.length ? Math.max(...ys) + NODE_H + PADDING * 2 : 200;
  });

  function handleNodeClick(task: TaskNode) {
    if (task.agentId) {
      openPane("agent-log", task.agentName ?? "agent", { agentId: task.agentId });
    }
  }

  return (
    <svg
      width={svgWidth()}
      height={svgHeight()}
      class="min-w-full"
      viewBox={`0 0 ${svgWidth()} ${svgHeight()}`}
    >
      {/* Edges */}
      <For each={layout()}>
        {(node) => (
          <For each={node.task.dependsOn ?? []}>
            {(depId) => {
              const from = posMap().get(depId);
              if (!from) return null;
              const x1 = from.x + NODE_W;
              const y1 = from.y + NODE_H / 2;
              const x2 = node.x;
              const y2 = node.y + NODE_H / 2;
              const cx = (x1 + x2) / 2;
              return (
                <path
                  d={`M ${x1} ${y1} C ${cx} ${y1}, ${cx} ${y2}, ${x2} ${y2}`}
                  fill="none"
                  stroke="#374151"
                  stroke-width="1.5"
                  stroke-dasharray={node.task.status === "pending" ? "4 4" : undefined}
                />
              );
            }}
          </For>
        )}
      </For>

      {/* Nodes */}
      <For each={layout()}>
        {(node) => {
          const colors = () => STATUS_COLORS[node.task.status] ?? STATUS_COLORS.pending;
          return (
            <g
              class="cursor-pointer"
              onClick={() => handleNodeClick(node.task)}
            >
              {/* Background */}
              <rect
                x={node.x}
                y={node.y}
                width={NODE_W}
                height={NODE_H}
                rx="8"
                fill={colors().fill}
                stroke={colors().stroke}
                stroke-width="1.5"
              />

              {/* Status dot */}
              <circle
                cx={node.x + 14}
                cy={node.y + NODE_H / 2}
                r="4"
                fill={colors().stroke}
              />

              {/* Title (truncated) */}
              <text
                x={node.x + 26}
                y={node.y + 22}
                fill={colors().text}
                font-size="11"
                font-weight="600"
                font-family="ui-monospace, monospace"
              >
                {node.task.title.length > 20
                  ? node.task.title.slice(0, 20) + "..."
                  : node.task.title}
              </text>

              {/* Agent badge */}
              <text
                x={node.x + 26}
                y={node.y + 40}
                fill="#6b7280"
                font-size="9"
                font-family="ui-monospace, monospace"
              >
                {node.task.agentName ?? node.task.status}
              </text>
            </g>
          );
        }}
      </For>
    </svg>
  );
};

export default TaskDAG;
export type { TaskNode };
