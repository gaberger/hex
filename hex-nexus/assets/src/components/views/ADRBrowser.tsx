import { Component, createSignal, createResource, For, Show, createMemo } from 'solid-js';
import { navigate, route } from '../../stores/router';
import MarkdownContent from '../chat/MarkdownContent';

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

async function fetchADRList(): Promise<ADRListItem[]> {
  try {
    const res = await fetch('/api/adrs');
    if (res.ok) {
      return await res.json();
    }
  } catch {
    // API not available, fall through to fallback
  }

  // TODO: Replace with real API data when /api/adrs is available
  return [
    { id: '041', title: 'SpacetimeDB Single State', status: 'Proposed', date: '2026-03-21' },
    { id: '040', title: 'Dashboard Redesign', status: 'Accepted', date: '2026-03-19' },
    { id: '039', title: 'Control Plane Architecture', status: 'Accepted', date: '2026-03-18' },
    { id: '035', title: 'Inference Gateway', status: 'Accepted', date: '2026-03-10' },
    { id: '027', title: 'HexFlo Coordination', status: 'Accepted', date: '2026-02-15' },
    { id: '025', title: 'SpacetimeDB Integration', status: 'Accepted', date: '2026-02-10' },
    { id: '014', title: 'Deps Pattern (No Mocks)', status: 'Superseded', date: '2025-12-01' },
    { id: '011', title: 'Multi-Instance Coordination', status: 'Accepted', date: '2025-11-15' },
  ];
}

async function fetchADRDetail(id: string): Promise<ADRDetail | null> {
  if (!id) return null;
  try {
    const res = await fetch(`/api/adrs/${id}`);
    if (res.ok) {
      return await res.json();
    }
  } catch {
    // API not available, fall through to fallback
  }

  // TODO: Replace with real API data when /api/adrs/:id is available
  return {
    id,
    title: `ADR-${id}`,
    status: 'Proposed',
    date: '2026-03-21',
    drivers: 'Pending API integration',
    content: `# ADR-${id}\n\n*Content will be loaded from the API once \`/api/adrs/${id}\` is available.*\n\nRun \`hex adr status ${id}\` from the CLI to view this ADR.`,
  };
}

const ADRBrowser: Component = () => {
  const [searchQuery, setSearchQuery] = createSignal('');
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [showRaw, setShowRaw] = createSignal(false);

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
            const meta = () => selectedADR();

            return (
              <>
                {/* Title bar */}
                <div class="flex items-center gap-3 border-b border-gray-800 px-6 py-4">
                  <svg
                    class="h-5 w-5 shrink-0 text-gray-500"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                  >
                    <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                    <polyline points="14 2 14 8 20 8" />
                    <line x1="16" y1="13" x2="8" y2="13" />
                    <line x1="16" y1="17" x2="8" y2="17" />
                    <polyline points="10 9 9 9 8 9" />
                  </svg>
                  <h1 class="flex-1 text-base font-bold text-[#e5e7eb]">
                    ADR-{detail().id}: {detail().title}
                  </h1>
                  <div class="flex items-center gap-2">
                    <button
                      class="rounded-md border border-gray-700 px-3 py-1 text-xs text-gray-400 hover:bg-gray-800 hover:text-gray-200 transition-colors"
                      onClick={() => {
                        // TODO: Open in editor
                      }}
                    >
                      Edit
                    </button>
                    <button
                      class="rounded-md border border-gray-700 px-3 py-1 text-xs transition-colors"
                      classList={{
                        'bg-gray-800 text-gray-200': showRaw(),
                        'text-gray-400 hover:bg-gray-800 hover:text-gray-200': !showRaw(),
                      }}
                      onClick={() => setShowRaw(!showRaw())}
                    >
                      Raw
                    </button>
                  </div>
                </div>

                {/* Metadata bar */}
                <div class="flex flex-wrap items-center gap-4 bg-[#111827] px-6 py-3 text-xs">
                  <div class="flex items-center gap-2">
                    <span class="text-gray-500">Status</span>
                    <span
                      class={`rounded px-2 py-0.5 font-medium ${statusBadgeClasses(meta()?.status ?? detail().status)}`}
                    >
                      {meta()?.status ?? detail().status}
                    </span>
                  </div>
                  <Show when={detail().date || meta()?.date}>
                    <div class="flex items-center gap-2">
                      <span class="text-gray-500">Date</span>
                      <span class="text-gray-300">{detail().date ?? meta()?.date}</span>
                    </div>
                  </Show>
                  <Show when={detail().drivers}>
                    <div class="flex items-center gap-2">
                      <span class="text-gray-500">Drivers</span>
                      <span class="text-gray-300">{detail().drivers}</span>
                    </div>
                  </Show>
                </div>

                {/* Content */}
                <div class="flex-1 overflow-y-auto px-6 py-6">
                  <Show
                    when={!showRaw()}
                    fallback={
                      <pre
                        class="whitespace-pre-wrap text-sm text-[#d1d5db] leading-relaxed"
                        style={{ 'font-family': "'JetBrains Mono', monospace", 'font-size': '14px' }}
                      >
                        {detail().content}
                      </pre>
                    }
                  >
                    <div
                      class="adr-markdown-content"
                      style={{
                        '--md-heading-size': '16px',
                        '--md-body-size': '14px',
                      } as any}
                    >
                      <MarkdownContent content={detail().content} />
                    </div>
                  </Show>
                </div>
              </>
            );
          })()}
        </Show>
      </div>
    </div>
  );
};

export default ADRBrowser;
