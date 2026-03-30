/**
 * DependencyGraphPane.tsx — Canvas-based force-directed dependency graph.
 *
 * Visualizes hex architecture layers and import relationships.
 * Data comes from POST /api/analyze with include_graph: true.
 */
import { Component, onMount, onCleanup, createSignal, Show } from 'solid-js';
import { restClient } from '../../services/rest-client';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface GraphNode {
  id: string;
  label: string;
  layer: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  pinned: boolean;
}

interface GraphEdge {
  from: string;
  to: string;
  isViolation: boolean;
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const LAYER_COLORS: Record<string, string> = {
  domain: '#22c55e',       // green
  ports: '#bc8cff',        // purple
  usecases: '#eab308',     // yellow
  'use-cases': '#eab308',  // yellow (alias)
  'adapters/primary': '#f97316',   // orange
  'adapters/secondary': '#3b82f6', // blue
  primary: '#f97316',      // orange
  secondary: '#3b82f6',    // blue
  external: '#8b949e',
  infrastructure: '#6e7a88',
};

const LAYER_LABELS: Record<string, string> = {
  domain: 'Domain',
  ports: 'Ports',
  usecases: 'Use Cases',
  primary: 'Primary Adapters',
  secondary: 'Secondary Adapters',
  external: 'External',
  infrastructure: 'Infrastructure',
};

const NODE_RADIUS = 8;
const LABEL_OFFSET = 12;
const MAX_ITERATIONS = 100;
const REPULSION = 800;
const ATTRACTION = 0.005;
const CENTERING = 0.01;
const DAMPING = 0.9;
const VELOCITY_THRESHOLD = 0.01;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function getLayerFromPath(path: string): string {
  if (path.includes('/domain/')) return 'domain';
  if (path.includes('/ports/')) return 'ports';
  if (path.includes('/usecases/') || path.includes('/use-cases/')) return 'usecases';
  if (path.includes('/primary/')) return 'primary';
  if (path.includes('/secondary/')) return 'secondary';
  if (path.includes('/infrastructure/')) return 'infrastructure';
  if (path.includes('node_modules') || !path.startsWith('.')) return 'external';
  return 'external';
}

function truncateLabel(path: string, maxLen = 20): string {
  const parts = path.split('/');
  const name = parts[parts.length - 1] ?? path;
  return name.length > maxLen ? name.slice(0, maxLen - 1) + '\u2026' : name;
}

function colorForLayer(layer: string): string {
  return LAYER_COLORS[layer] ?? LAYER_COLORS.external;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

const DependencyGraphPane: Component = () => {
  let canvasRef: HTMLCanvasElement | undefined;
  let containerRef: HTMLDivElement | undefined;
  let animFrameId = 0;
  let resizeObs: ResizeObserver | undefined;

  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [nodes, setNodes] = createSignal<GraphNode[]>([]);
  const [edges, setEdges] = createSignal<GraphEdge[]>([]);
  const [iteration, setIteration] = createSignal(0);

  // Camera state
  let zoom = 1;
  let panX = 0;
  let panY = 0;

  // Drag state
  let dragNode: GraphNode | null = null;
  let isPanning = false;
  let lastMouseX = 0;
  let lastMouseY = 0;

  // ---------------------------------------------------------------------------
  // Data fetching
  // ---------------------------------------------------------------------------

  async function fetchGraph() {
    setLoading(true);
    setError(null);
    try {
      const data = await restClient.post('/api/analyze', { root_path: '.', include_graph: true });
      buildGraph(data);
    } catch (e: any) {
      setError(e?.message ?? 'Failed to fetch');
    } finally {
      setLoading(false);
    }
  }

  function buildGraph(data: any) {
    const files: any[] = data.files ?? [];
    if (files.length === 0) {
      setError('No files returned from analysis');
      return;
    }

    const width = canvasRef?.width ?? 800;
    const height = canvasRef?.height ?? 600;

    // Build nodes
    const nodeMap = new Map<string, GraphNode>();
    files.forEach((f: any, i: number) => {
      const path = f.path ?? f.file ?? '';
      const layer = f.layer ?? getLayerFromPath(path);
      const angle = (2 * Math.PI * i) / files.length;
      const radius = Math.min(width, height) * 0.3;
      nodeMap.set(path, {
        id: path,
        label: truncateLabel(path),
        layer,
        x: width / 2 + radius * Math.cos(angle) + (Math.random() - 0.5) * 20,
        y: height / 2 + radius * Math.sin(angle) + (Math.random() - 0.5) * 20,
        vx: 0,
        vy: 0,
        pinned: false,
      });
    });

    // Build edges
    const graphEdges: GraphEdge[] = [];

    if (data.edges && Array.isArray(data.edges)) {
      for (const e of data.edges) {
        const from = e.from ?? e.source;
        const to = e.to ?? e.target;
        if (nodeMap.has(from) && nodeMap.has(to)) {
          graphEdges.push({
            from,
            to,
            isViolation: e.kind === 'violation' || e.is_violation === true,
          });
        }
      }
    } else {
      // Derive edges from imports
      for (const f of files) {
        const path = f.path ?? f.file ?? '';
        const imports: string[] = f.imports ?? [];
        for (const imp of imports) {
          if (nodeMap.has(imp)) {
            graphEdges.push({ from: path, to: imp, isViolation: false });
          }
        }
      }
    }

    setNodes(Array.from(nodeMap.values()));
    setEdges(graphEdges);
    setIteration(0);

    // Reset camera
    zoom = 1;
    panX = 0;
    panY = 0;

    startSimulation();
  }

  // ---------------------------------------------------------------------------
  // Force simulation
  // ---------------------------------------------------------------------------

  function startSimulation() {
    cancelAnimationFrame(animFrameId);
    setIteration(0);
    tick();
  }

  function tick() {
    const currentNodes = nodes();
    const currentEdges = edges();
    const iter = iteration();

    if (iter >= MAX_ITERATIONS || currentNodes.length === 0) {
      // Final render
      render();
      return;
    }

    const width = canvasRef?.width ?? 800;
    const height = canvasRef?.height ?? 600;
    const cx = width / 2;
    const cy = height / 2;

    // Reset forces
    for (const n of currentNodes) {
      if (n.pinned) continue;
      n.vx = 0;
      n.vy = 0;
    }

    // Repulsion (all pairs)
    for (let i = 0; i < currentNodes.length; i++) {
      for (let j = i + 1; j < currentNodes.length; j++) {
        const a = currentNodes[i];
        const b = currentNodes[j];
        let dx = b.x - a.x;
        let dy = b.y - a.y;
        let dist = Math.sqrt(dx * dx + dy * dy);
        if (dist < 1) dist = 1;
        const force = REPULSION / (dist * dist);
        const fx = (dx / dist) * force;
        const fy = (dy / dist) * force;
        if (!a.pinned) { a.vx -= fx; a.vy -= fy; }
        if (!b.pinned) { b.vx += fx; b.vy += fy; }
      }
    }

    // Attraction (edges)
    const nodeIndex = new Map<string, GraphNode>();
    for (const n of currentNodes) nodeIndex.set(n.id, n);

    for (const e of currentEdges) {
      const a = nodeIndex.get(e.from);
      const b = nodeIndex.get(e.to);
      if (!a || !b) continue;
      const dx = b.x - a.x;
      const dy = b.y - a.y;
      const dist = Math.sqrt(dx * dx + dy * dy);
      const force = dist * ATTRACTION;
      const fx = dx * force;
      const fy = dy * force;
      if (!a.pinned) { a.vx += fx; a.vy += fy; }
      if (!b.pinned) { b.vx -= fx; b.vy -= fy; }
    }

    // Centering + apply
    let maxV = 0;
    for (const n of currentNodes) {
      if (n.pinned) continue;
      n.vx += (cx - n.x) * CENTERING;
      n.vy += (cy - n.y) * CENTERING;
      n.vx *= DAMPING;
      n.vy *= DAMPING;
      n.x += n.vx;
      n.y += n.vy;
      maxV = Math.max(maxV, Math.abs(n.vx), Math.abs(n.vy));
    }

    setNodes([...currentNodes]);
    setIteration(iter + 1);

    render();

    if (maxV > VELOCITY_THRESHOLD && iter < MAX_ITERATIONS) {
      animFrameId = requestAnimationFrame(tick);
    }
  }

  // ---------------------------------------------------------------------------
  // Rendering
  // ---------------------------------------------------------------------------

  function render() {
    const canvas = canvasRef;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const w = canvas.width;
    const h = canvas.height;
    const dpr = window.devicePixelRatio || 1;

    ctx.clearRect(0, 0, w, h);
    ctx.save();
    ctx.scale(dpr, dpr);

    const lw = w / dpr;
    const lh = h / dpr;

    // Apply camera transform
    ctx.translate(lw / 2, lh / 2);
    ctx.scale(zoom, zoom);
    ctx.translate(-lw / 2 + panX, -lh / 2 + panY);

    const currentNodes = nodes();
    const currentEdges = edges();
    const nodeIndex = new Map<string, GraphNode>();
    for (const n of currentNodes) nodeIndex.set(n.id, n);

    // Draw edges
    for (const e of currentEdges) {
      const a = nodeIndex.get(e.from);
      const b = nodeIndex.get(e.to);
      if (!a || !b) continue;

      ctx.beginPath();
      ctx.moveTo(a.x, a.y);
      ctx.lineTo(b.x, b.y);
      ctx.strokeStyle = e.isViolation ? '#f85149' : 'rgba(139,148,158,0.3)';
      ctx.lineWidth = e.isViolation ? 2 : 1;
      ctx.stroke();

      // Arrowhead
      const angle = Math.atan2(b.y - a.y, b.x - a.x);
      const arrowLen = 8;
      const tipX = b.x - (NODE_RADIUS + 2) * Math.cos(angle);
      const tipY = b.y - (NODE_RADIUS + 2) * Math.sin(angle);

      ctx.beginPath();
      ctx.moveTo(tipX, tipY);
      ctx.lineTo(
        tipX - arrowLen * Math.cos(angle - Math.PI / 7),
        tipY - arrowLen * Math.sin(angle - Math.PI / 7),
      );
      ctx.lineTo(
        tipX - arrowLen * Math.cos(angle + Math.PI / 7),
        tipY - arrowLen * Math.sin(angle + Math.PI / 7),
      );
      ctx.closePath();
      ctx.fillStyle = e.isViolation ? '#f85149' : 'rgba(139,148,158,0.5)';
      ctx.fill();
    }

    // Draw nodes
    for (const n of currentNodes) {
      ctx.beginPath();
      ctx.arc(n.x, n.y, NODE_RADIUS, 0, Math.PI * 2);
      ctx.fillStyle = colorForLayer(n.layer);
      ctx.fill();
      ctx.strokeStyle = 'rgba(0,0,0,0.4)';
      ctx.lineWidth = 1;
      ctx.stroke();

      // Label
      ctx.fillStyle = '#c9d1d9';
      ctx.font = '10px Inter, system-ui, sans-serif';
      ctx.textAlign = 'center';
      ctx.fillText(n.label, n.x, n.y + NODE_RADIUS + LABEL_OFFSET);
    }

    ctx.restore();
  }

  // ---------------------------------------------------------------------------
  // Mouse interaction
  // ---------------------------------------------------------------------------

  function screenToWorld(sx: number, sy: number): [number, number] {
    const canvas = canvasRef;
    if (!canvas) return [sx, sy];
    const dpr = window.devicePixelRatio || 1;
    const lw = canvas.width / dpr;
    const lh = canvas.height / dpr;
    const wx = (sx - lw / 2) / zoom + lw / 2 - panX;
    const wy = (sy - lh / 2) / zoom + lh / 2 - panY;
    return [wx, wy];
  }

  function findNodeAt(wx: number, wy: number): GraphNode | null {
    const hitRadius = NODE_RADIUS + 4;
    for (const n of nodes()) {
      const dx = n.x - wx;
      const dy = n.y - wy;
      if (dx * dx + dy * dy < hitRadius * hitRadius) return n;
    }
    return null;
  }

  function handleMouseDown(e: MouseEvent) {
    const rect = canvasRef?.getBoundingClientRect();
    if (!rect) return;
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;
    const [wx, wy] = screenToWorld(sx, sy);
    const hit = findNodeAt(wx, wy);

    if (hit) {
      dragNode = hit;
      hit.pinned = true;
      lastMouseX = sx;
      lastMouseY = sy;
    } else {
      isPanning = true;
      lastMouseX = sx;
      lastMouseY = sy;
    }
  }

  function handleMouseMove(e: MouseEvent) {
    const rect = canvasRef?.getBoundingClientRect();
    if (!rect) return;
    const sx = e.clientX - rect.left;
    const sy = e.clientY - rect.top;

    if (dragNode) {
      const dx = (sx - lastMouseX) / zoom;
      const dy = (sy - lastMouseY) / zoom;
      dragNode.x += dx;
      dragNode.y += dy;
      lastMouseX = sx;
      lastMouseY = sy;
      setNodes([...nodes()]);
      render();
    } else if (isPanning) {
      panX += (sx - lastMouseX) / zoom;
      panY += (sy - lastMouseY) / zoom;
      lastMouseX = sx;
      lastMouseY = sy;
      render();
    }
  }

  function handleMouseUp() {
    if (dragNode) {
      dragNode.pinned = false;
      dragNode = null;
    }
    isPanning = false;
  }

  function handleWheel(e: WheelEvent) {
    e.preventDefault();
    const factor = e.deltaY > 0 ? 0.9 : 1.1;
    zoom = Math.max(0.1, Math.min(5, zoom * factor));
    render();
  }

  // Zoom controls
  function zoomIn() { zoom = Math.min(5, zoom * 1.2); render(); }
  function zoomOut() { zoom = Math.max(0.1, zoom / 1.2); render(); }
  function zoomReset() { zoom = 1; panX = 0; panY = 0; render(); }

  // ---------------------------------------------------------------------------
  // Resize handling
  // ---------------------------------------------------------------------------

  function resizeCanvas() {
    const canvas = canvasRef;
    const container = containerRef;
    if (!canvas || !container) return;
    const dpr = window.devicePixelRatio || 1;
    const rect = container.getBoundingClientRect();
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    canvas.style.width = `${rect.width}px`;
    canvas.style.height = `${rect.height}px`;
    render();
  }

  // ---------------------------------------------------------------------------
  // Lifecycle
  // ---------------------------------------------------------------------------

  onMount(() => {
    resizeCanvas();

    resizeObs = new ResizeObserver(() => resizeCanvas());
    if (containerRef) resizeObs.observe(containerRef);

    fetchGraph();
  });

  onCleanup(() => {
    cancelAnimationFrame(animFrameId);
    resizeObs?.disconnect();
  });

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  return (
    <div ref={containerRef} class="relative flex h-full w-full flex-col bg-gray-950">
      {/* Toolbar */}
      <div class="flex shrink-0 items-center gap-2 border-b border-gray-800 bg-gray-900/80 px-3 py-1.5">
        <span class="text-[11px] font-semibold uppercase tracking-wider text-gray-500">
          Dependency Graph
        </span>
        <Show when={iteration() > 0 && iteration() < MAX_ITERATIONS}>
          <span class="text-[10px] text-gray-600">
            simulating ({iteration()}/{MAX_ITERATIONS})
          </span>
        </Show>
        <div class="ml-auto flex items-center gap-1">
          <button
            class="rounded border border-gray-700 px-2 py-0.5 text-[10px] text-gray-400 hover:border-cyan-600 hover:text-cyan-300 transition-colors"
            onClick={fetchGraph}
            disabled={loading()}
          >
            {loading() ? 'Loading...' : 'Refresh'}
          </button>
        </div>
      </div>

      {/* Canvas area */}
      <div class="relative flex-1 overflow-hidden">
        <Show when={loading()}>
          <div class="absolute inset-0 z-10 flex items-center justify-center bg-gray-950/80">
            <span class="text-sm text-gray-400">Analyzing...</span>
          </div>
        </Show>

        <Show when={error()}>
          <div class="absolute inset-0 z-10 flex items-center justify-center bg-gray-950/80">
            <div class="text-center">
              <p class="text-sm text-red-400">{error()}</p>
              <button
                class="mt-2 rounded border border-gray-700 px-3 py-1 text-xs text-gray-400 hover:border-cyan-600 hover:text-cyan-300 transition-colors"
                onClick={fetchGraph}
              >
                Retry
              </button>
            </div>
          </div>
        </Show>

        <canvas
          ref={canvasRef}
          class="block h-full w-full cursor-grab active:cursor-grabbing"
          onMouseDown={handleMouseDown}
          onMouseMove={handleMouseMove}
          onMouseUp={handleMouseUp}
          onMouseLeave={handleMouseUp}
          onWheel={handleWheel}
        />

        {/* Zoom controls */}
        <div class="absolute right-3 top-3 flex flex-col gap-1">
          <button
            class="flex h-7 w-7 items-center justify-center rounded border border-gray-700 bg-gray-900/90 text-sm text-gray-400 hover:border-cyan-600 hover:text-cyan-300 transition-colors"
            onClick={zoomIn}
            title="Zoom in"
          >+</button>
          <button
            class="flex h-7 w-7 items-center justify-center rounded border border-gray-700 bg-gray-900/90 text-sm text-gray-400 hover:border-cyan-600 hover:text-cyan-300 transition-colors"
            onClick={zoomOut}
            title="Zoom out"
          >-</button>
          <button
            class="flex h-7 w-7 items-center justify-center rounded border border-gray-700 bg-gray-900/90 text-[9px] text-gray-400 hover:border-cyan-600 hover:text-cyan-300 transition-colors"
            onClick={zoomReset}
            title="Reset zoom"
          >1:1</button>
        </div>
      </div>

      {/* Legend */}
      <div class="flex shrink-0 flex-wrap items-center gap-x-4 gap-y-1 border-t border-gray-800 bg-gray-900/80 px-3 py-1.5">
        {Object.entries(LAYER_LABELS).map(([key, label]) => (
          <div class="flex items-center gap-1.5">
            <span
              class="inline-block h-2.5 w-2.5 rounded-full"
              style={{ background: LAYER_COLORS[key] ?? '#8b949e' }}
            />
            <span class="text-[10px] text-gray-400">{label}</span>
          </div>
        ))}
        <div class="flex items-center gap-1.5">
          <span class="inline-block h-0.5 w-4 rounded bg-[#f85149]" />
          <span class="text-[10px] text-red-400">Violation</span>
        </div>
      </div>
    </div>
  );
};

export default DependencyGraphPane;
