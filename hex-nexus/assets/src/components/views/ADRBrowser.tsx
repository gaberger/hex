import { Component, createSignal, createResource, For, Show, createMemo } from 'solid-js';
import { navigate, route } from '../../stores/router';
import { MarkdownEditor } from '../editor';
import { addToast } from '../../stores/toast';

interface ADRListItem {
  id: string;
  title: string;
  status: string;
  date?: string;
}

interface ADRDetail {
  id: string;
  title: string;
  status: string;
  date?: string;
  drivers?: string;
  content: string;
}

function statusBadgeClasses(status: string): string {
  const s = status.toLowerCase();
  if (s === 'proposed') return 'bg-yellow-500/15 text-yellow-400 border border-yellow-500/30';
  if (s === 'accepted') return 'bg-green-500/15 text-green-400 border border-green-500/30';
  if (s === 'superseded') return 'bg-red-500/15 text-red-400 border border-red-500/30';
  if (s === 'deprecated') return 'bg-red-500/15 text-red-400 border border-red-500/30';
  if (s === 'abandoned') return 'bg-gray-500/15 text-gray-400 border border-gray-500/30';
  return 'bg-gray-500/15 text-gray-400 border border-gray-500/30';
}

function statusColor(status: string): string {
  const s = status.toLowerCase();
  if (s === 'proposed') return '#eab308';
  if (s === 'accepted') return '#4ade80';
  if (s === 'superseded') return '#f87149';
  return '#6b7280';
}

async function fetchADRList(): Promise<ADRListItem[]> {
  try {
    const res = await fetch('/api/adrs');
    if (res.ok) {
      return await res.json();
    }
  } catch {
    // API not available, fall through to fallback
  }

  // Fallback data for known ADRs when API is unavailable
  return [
    { id: '043', title: 'AIIDE - Hex Nexus as AI IDE', status: 'Accepted', date: '2026-03-21' },
    { id: '042', title: 'SpacetimeDB Single Source of State', status: 'Proposed', date: '2026-03-21' },
    { id: '041', title: 'ADR Review Agent', status: 'Proposed', date: '2026-03-21' },
    { id: '040', title: 'Dashboard Redesign', status: 'Accepted', date: '2026-03-19' },
    { id: '039', title: 'Control Plane Architecture', status: 'Accepted', date: '2026-03-18' },
    { id: '035', title: 'Inference Gateway', status: 'Accepted', date: '2026-03-10' },
    { id: '027', title: 'HexFlo Coordination', status: 'Accepted', date: '2026-02-15' },
    { id: '025', title: 'SpacetimeDB Integration', status: 'Accepted', date: '2026-02-10' },
    { id: '014', title: 'Deps Pattern (No Mocks)', status: 'Superseded', date: '2025-12-01' },
    { id: '011', title: 'Multi-Instance Coordination', status: 'Accepted', date: '2025-11-15' },
  ];
}

// Fallback markdown content for key ADRs (embedded from docs/adrs/)
const ADR_FALLBACK_CONTENT: Record<string, { title: string; status: string; date: string; drivers?: string; content: string }> = {
  '043': {
    title: 'AIIDE - Hex Nexus as AI IDE',
    status: 'Accepted',
    date: '2026-03-21',
    drivers: 'Dashboard redesign, OpenCode UX research, agent fleet management needs',
    content: `# ADR-043: AIIDE — Hex Nexus as AI Integrated Development Environment

**Status:** Accepted
**Date:** 2026-03-21
**Drivers:** Dashboard redesign, OpenCode UX research, agent fleet management needs

## Context

Hex Nexus evolved from a monitoring dashboard into something that doesn't fit existing categories. It is not an IDE with AI features (Cursor, Copilot). It is not a chat app with code execution (ChatGPT). It is not a DevOps dashboard (Grafana). It is a purpose-built environment where AI agents are the primary developers and humans provide guidance.

We coin the term **AIIDE** (AI Integrated Development Environment, pronounced "aide") to describe this new category.

## Decision

Hex Nexus is an AIIDE. Its design principles are:

### 1. Five Pillars
- **Projects** — isolated worktrees, hex architecture analysis, dependency graphs
- **Agents** — local (Claude Code, hex-coder) and remote (hex-agent over SSH), with lifecycle management
- **Swarms** — HexFlo multi-agent coordination with task tracking and progress visualization
- **Inference** — multi-provider management (Ollama, OpenAI, Anthropic) with health, cost, token budget
- **Configuration** — architecture blueprints, MCP tools, hooks, skills, CLAUDE.md context, agent definitions

### 2. Navigation Model
Breadcrumb-based hierarchical navigation with Control Plane as root.

### 3. State Architecture
- SpacetimeDB is the single source of truth for ALL coordination state (ADR-042)
- hex-nexus binary is stateless compute (filesystem, processes, outbound HTTP)
- Dashboard connects directly to SpacetimeDB via WebSocket subscriptions

### 4. Chat Scoping
- Control Plane chat = manage infrastructure
- Project chat = develop code scoped to that project
- Plan mode (blue) = discuss, no side effects
- Build mode (green) = execute changes`,
  },
  '042': {
    title: 'SpacetimeDB Single Source of State',
    status: 'Proposed',
    date: '2026-03-21',
    drivers: 'UX dashboard redesign revealed state fragmentation across 4+ backends',
    content: `# ADR-042: SpacetimeDB as Single Source of State

**Status:** Proposed
**Date:** 2026-03-21
**Drivers:** UX dashboard redesign revealed state fragmentation across 4+ backends

## Context

The hex-nexus system currently stores coordination state in **multiple disconnected backends**:

| Backend | What it stores |
|---------|---------------|
| \`hex-nexus\` SpacetimeDB database | Swarms, tasks, agents, memory |
| \`hexflo-coordination\` SpacetimeDB database | Swarms, tasks, agents, memory |
| \`inference-gateway\` SpacetimeDB database | Inference providers, requests |
| \`agent-registry\` SpacetimeDB database | Agent heartbeats, status |
| In-memory \`HashMap\` (Rust) | Inference endpoints, instances |
| SQLite \`hub.db\` | Chat sessions, project registry |

**Problem:** The dashboard subscribes to \`hexflo-coordination\` but CLI/MCP writes to \`hex-nexus\`. These are different databases with the same schema but different data.

## Decision

**Consolidate ALL coordination state into the 4 canonical SpacetimeDB modules:**

1. \`hexflo-coordination\` — swarms, tasks, agents, memory
2. \`inference-gateway\` — providers, requests, budgets, streaming
3. \`agent-registry\` — agent lifecycle, heartbeats
4. \`fleet-state\` — compute nodes

**hex-nexus binary becomes stateless compute:** filesystem operations, process management, outbound HTTP, static asset serving, and WebSocket proxy for chat.`,
  },
  '041': {
    title: 'ADR Review Agent',
    status: 'Proposed',
    date: '2026-03-21',
    content: `# ADR-041: ADR Review Agent

**Status:** Proposed
**Date:** 2026-03-21

## Context

Architectural consistency is hard to maintain when multiple AI agents contribute code changes. ADRs define decisions, but nothing enforces that code changes respect them.

## Decision

Introduce an ADR Review Agent — an architectural consistency guardian that reviews code changes against ADR decisions. It runs as a validation gate before merges and flags violations.`,
  },
  '027': {
    title: 'HexFlo Coordination',
    status: 'Accepted',
    date: '2026-02-15',
    content: `# ADR-027: HexFlo — Native Rust Coordination Layer

**Status:** Accepted
**Date:** 2026-02-15

## Context

Swarm coordination previously relied on external tools. Native Rust coordination eliminates external dependencies and provides tighter integration with hex-nexus.

## Decision

HexFlo is the native Rust coordination layer built into hex-nexus. It provides swarm init, task create/complete, memory store/retrieve, and heartbeat-based agent lifecycle management. All state persists in SpacetimeDB.`,
  },
  '025': {
    title: 'SpacetimeDB Integration',
    status: 'Accepted',
    date: '2026-02-10',
    content: `# ADR-025: SpacetimeDB Integration

**Status:** Accepted
**Date:** 2026-02-10

## Context

hex-nexus needs real-time state synchronization between CLI, dashboard, and agents. Traditional REST polling creates lag and complexity.

## Decision

Adopt SpacetimeDB as the real-time state backend. Its WebSocket subscriptions provide instant propagation of state changes to all connected clients.`,
  },
};

async function fetchADRContent(adrId: string): Promise<string> {
  // Try dedicated ADR content endpoint
  try {
    const res = await fetch(`/api/adrs/${adrId}/content`);
    if (res.ok) {
      const data = await res.json();
      return data.content || data.body || '';
    }
  } catch { /* fall through */ }

  // Try file read via projects API
  try {
    const res = await fetch(`/api/projects/hex-intf/files?path=docs/adrs/adr-${adrId}*.md`);
    if (res.ok) {
      const data = await res.json();
      if (data.content) return data.content;
    }
  } catch { /* fall through */ }

  // Use embedded fallback content
  return ADR_FALLBACK_CONTENT[adrId]?.content
    || `# ADR-${adrId}\n\n*Content will be loaded from the API once available.*\n\nRun \`hex adr status ${adrId}\` from the CLI to view this ADR.`;
}

async function fetchADRDetail(id: string): Promise<ADRDetail | null> {
  if (!id) return null;

  // Try the full detail API first
  try {
    const res = await fetch(`/api/adrs/${id}`);
    if (res.ok) {
      return await res.json();
    }
  } catch {
    // API not available, fall through
  }

  // Try to get content from filesystem via nexus
  const content = await fetchADRContent(id);

  // Use fallback metadata if available, otherwise generic
  const fallback = ADR_FALLBACK_CONTENT[id];
  return {
    id,
    title: fallback?.title || `ADR-${id}`,
    status: fallback?.status || 'Proposed',
    date: fallback?.date || '2026-03-21',
    drivers: fallback?.drivers || undefined,
    content,
  };
}

const ADRBrowser: Component = () => {
  const [searchQuery, setSearchQuery] = createSignal('');
  const [selectedId, setSelectedId] = createSignal<string | null>(null);

  const [adrList] = createResource(fetchADRList);

  const filteredList = createMemo(() => {
    const list = adrList() ?? [];
    const q = searchQuery().toLowerCase().trim();
    if (!q) return list;
    return list.filter(
      (adr) =>
        adr.id.includes(q) ||
        adr.title.toLowerCase().includes(q) ||
        adr.status.toLowerCase().includes(q)
    );
  });

  // Auto-select first ADR when list loads
  const effectiveSelectedId = createMemo(() => {
    const sel = selectedId();
    if (sel) return sel;
    const list = filteredList();
    return list.length > 0 ? list[0].id : null;
  });

  const [adrDetail] = createResource(effectiveSelectedId, fetchADRDetail);

  const selectedADR = createMemo(() => {
    const list = adrList() ?? [];
    return list.find((a) => a.id === effectiveSelectedId()) ?? null;
  });

  return (
    <div class="flex flex-1 overflow-hidden">
      {/* LEFT SIDEBAR — ADR List (inside center content area) */}
      <div
        class="flex flex-col border-r border-gray-800 bg-gray-900 overflow-hidden"
        style={{ width: '280px', 'min-width': '280px' }}
      >
        {/* Header */}
        <div class="flex items-center justify-between px-4 pt-4 pb-2">
          <span class="text-sm font-bold uppercase tracking-wide text-gray-400">ADRs</span>
          <span class="text-xs text-gray-600 font-mono">
            {filteredList().length} records
          </span>
        </div>

        {/* Search */}
        <div class="px-3 pb-3">
          <div class="relative">
            <svg
              class="absolute left-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-gray-500"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
            >
              <circle cx="11" cy="11" r="8" />
              <line x1="21" y1="21" x2="16.65" y2="16.65" />
            </svg>
            <input
              type="text"
              placeholder="Search ADRs..."
              class="w-full rounded-lg border border-gray-700 bg-[#0a0e14] py-2 pl-9 pr-3 text-sm text-gray-300 placeholder-gray-600 outline-none focus:border-gray-500 transition-colors"
              value={searchQuery()}
              onInput={(e) => setSearchQuery(e.currentTarget.value)}
            />
          </div>
        </div>

        {/* ADR List */}
        <div class="flex-1 overflow-y-auto px-2 pb-2 space-y-0.5">
          <Show when={adrList.loading}>
            <div class="px-3 py-4 text-sm text-gray-600">Loading ADRs...</div>
          </Show>
          <Show when={!adrList.loading && filteredList().length === 0}>
            <div class="px-3 py-4 text-sm text-gray-600">No ADRs found</div>
          </Show>
          <For each={filteredList()}>
            {(adr) => {
              const isSelected = () => effectiveSelectedId() === adr.id;
              return (
                <button
                  class="flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left transition-colors"
                  classList={{
                    'bg-[#1f2937]': isSelected(),
                    'hover:bg-gray-800/50': !isSelected(),
                  }}
                  onClick={() => setSelectedId(adr.id)}
                >
                  {/* ADR number */}
                  <span
                    class="shrink-0 text-[13px] font-bold"
                    classList={{
                      'text-[#f0883e]': isSelected(),
                      'text-[#6b7280]': !isSelected(),
                    }}
                    style={{ 'font-family': "'JetBrains Mono', monospace" }}
                  >
                    {adr.id}
                  </span>

                  {/* Title */}
                  <span
                    class="flex-1 truncate text-[13px]"
                    classList={{
                      'text-[#e5e7eb]': isSelected(),
                      'text-[#9ca3af]': !isSelected(),
                    }}
                  >
                    {adr.title}
                  </span>

                  {/* Status badge */}
                  <span
                    class={`shrink-0 rounded px-1.5 py-0.5 text-[10px] font-medium ${statusBadgeClasses(adr.status)}`}
                  >
                    {adr.status}
                  </span>
                </button>
              );
            }}
          </For>
        </div>
      </div>

      {/* CENTER — Markdown viewer */}
      <div class="flex flex-1 flex-col overflow-hidden bg-gray-950">
        <Show
          when={adrDetail() && !adrDetail.loading}
          fallback={
            <div class="flex flex-1 items-center justify-center text-gray-600">
              <Show when={adrDetail.loading} fallback="Select an ADR to view">
                Loading...
              </Show>
            </div>
          }
        >
          {(() => {
            const detail = () => adrDetail()!;

            return (
              <MarkdownEditor
                content={detail().content}
                title={`ADR-${detail().id}: ${detail().title}`}
                filePath={`docs/adrs/ADR-${detail().id}-*.md`}
                initialMode="view"
                editable={true}
                metadata={[
                  { label: "Status", value: detail().status, color: statusColor(detail().status) },
                  { label: "Date", value: detail().date || "\u2014" },
                  { label: "Drivers", value: detail().drivers || "Pending API integration" },
                ]}
                onSave={async (content) => {
                  try {
                    const res = await fetch(`/api/adrs/${detail().id}`, {
                      method: 'PUT',
                      headers: { 'Content-Type': 'application/json' },
                      body: JSON.stringify({ content }),
                    });
                    if (res.ok) {
                      addToast('success', `ADR-${detail().id} saved`);
                    } else {
                      const data = await res.json().catch(() => ({}));
                      addToast('error', data.error || 'Save failed');
                    }
                  } catch {
                    addToast('error', 'Save failed — is nexus running?');
                  }
                }}
              />
            );
          })()}
        </Show>
      </div>
    </div>
  );
};

export default ADRBrowser;
