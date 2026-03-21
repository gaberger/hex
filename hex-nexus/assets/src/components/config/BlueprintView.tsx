import { Component, For } from 'solid-js';

interface LayerDef {
  name: string;
  color: string;
  imports: string;
  path: string;
}

const LAYERS: LayerDef[] = [
  { name: 'Domain',    color: '#58a6ff', imports: 'nothing',       path: 'src/core/domain/' },
  { name: 'Ports',     color: '#bc8cff', imports: 'domain',        path: 'src/core/ports/' },
  { name: 'Use Cases', color: '#3fb950', imports: 'domain, ports', path: 'src/core/usecases/' },
  { name: 'Primary',   color: '#f0883e', imports: 'ports',         path: 'src/adapters/primary/' },
  { name: 'Secondary', color: '#d29922', imports: 'ports',         path: 'src/adapters/secondary/' },
];

interface BoundaryRule {
  passing: boolean;
  text: string;
}

const RULES: BoundaryRule[] = [
  { passing: true,  text: 'Adapters must NEVER import other adapters' },
  { passing: true,  text: 'Domain must only import from domain' },
  { passing: true,  text: 'Ports may import from domain only' },
  { passing: true,  text: 'Use cases may import from domain and ports only' },
  { passing: false, text: '1 violation: scaffold-service.ts imports composition-root.js' },
];

const ShieldCheck: Component = () => (
  <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="#4ade80" stroke-width="2">
    <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
    <polyline points="9 12 11 14 15 10" />
  </svg>
);

const ShieldAlert: Component = () => (
  <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="#f87149" stroke-width="2">
    <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
    <line x1="12" y1="8" x2="12" y2="12" />
    <line x1="12" y1="16" x2="12.01" y2="16" />
  </svg>
);

const BlueprintView: Component = () => {
  return (
    <div class="flex-1 overflow-auto p-6">
      {/* Header */}
      <div class="flex items-center justify-between mb-6">
        <div>
          <h2 class="text-xl font-bold text-gray-100">Architecture Blueprint</h2>
          <p class="mt-1 text-sm text-gray-400">
            Hexagonal architecture layers and boundary enforcement rules.
          </p>
        </div>
        <button class="rounded-lg bg-gray-800 px-4 py-2 text-sm font-medium text-gray-300 hover:bg-gray-700 hover:text-gray-100 transition-colors border border-gray-700">
          Edit Blueprint
        </button>
      </div>

      {/* Layers */}
      <div class="mb-8">
        <h3 class="text-xs font-bold uppercase tracking-wider text-gray-500 mb-3">Layers</h3>
        <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-5 gap-3">
          <For each={LAYERS}>
            {(layer) => (
              <div
                class="rounded-lg p-3"
                style={{
                  "background-color": "#111827",
                  "border": `1px solid ${layer.color}40`,
                }}
              >
                <div class="text-sm font-bold mb-2" style={{ color: layer.color }}>
                  {layer.name}
                </div>
                <div class="text-xs text-gray-500 mb-1">
                  Imports: <span class="text-gray-400">{layer.imports}</span>
                </div>
                <div class="text-[11px] text-gray-600" style={{ "font-family": "'JetBrains Mono', monospace" }}>
                  {layer.path}
                </div>
              </div>
            )}
          </For>
        </div>
      </div>

      {/* Boundary Rules */}
      <div>
        <h3 class="text-xs font-bold uppercase tracking-wider text-gray-500 mb-3">Boundary Rules</h3>
        <div class="space-y-2">
          <For each={RULES}>
            {(rule) => (
              <div class="flex items-center gap-3 rounded-lg px-4 py-2.5" style={{ "background-color": "#111827" }}>
                {rule.passing ? <ShieldCheck /> : <ShieldAlert />}
                <span
                  class="text-sm"
                  classList={{
                    "text-gray-300": rule.passing,
                    "text-red-400": !rule.passing,
                  }}
                >
                  {rule.text}
                </span>
              </div>
            )}
          </For>
        </div>
      </div>
    </div>
  );
};

export default BlueprintView;
