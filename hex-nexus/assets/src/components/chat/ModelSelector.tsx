import { Component, For, Show, createSignal, createResource, onMount } from 'solid-js';
import { selectedModel, setSelectedModel } from '../../stores/chat';

interface InferenceEndpoint {
  id: string;
  name: string;
  provider_type: string;
  models: string[] | string;
  status: string;
}

interface ModelOption {
  value: string;
  label: string;
}

function parseModels(endpoint: InferenceEndpoint): string[] {
  const raw = endpoint.models;
  if (Array.isArray(raw)) return raw;
  if (typeof raw !== 'string') return [];
  try {
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed)) return parsed;
    return [String(parsed)];
  } catch {
    return raw.trim() ? [raw.trim()] : [];
  }
}

function isHealthy(endpoint: InferenceEndpoint): boolean {
  const s = endpoint.status?.toLowerCase() ?? '';
  return s === 'healthy' || s === 'online' || s === 'active';
}

async function fetchEndpoints(): Promise<ModelOption[]> {
  const res = await fetch('/api/inference/endpoints');
  if (!res.ok) return [];
  const endpoints: InferenceEndpoint[] = await res.json();
  const options: ModelOption[] = [];
  for (const ep of endpoints) {
    if (!isHealthy(ep)) continue;
    for (const model of parseModels(ep)) {
      options.push({
        value: `${model}@${ep.id}`,
        label: `${model} (${ep.name || ep.provider_type})`,
      });
    }
  }
  return options;
}

const ModelSelector: Component = () => {
  const [options] = createResource(fetchEndpoints);

  // Default to first available model when options load and nothing is selected
  const ensureDefault = () => {
    const opts = options();
    if (opts && opts.length > 0 && !selectedModel()) {
      setSelectedModel(opts[0].value);
    }
  };

  return (
    <Show
      when={options() && options()!.length > 0}
      fallback={
        <select
          disabled
          class="h-7 w-48 rounded-md border border-gray-700 bg-gray-800 px-2 text-xs text-gray-500 opacity-60 cursor-not-allowed"
        >
          <option>No models</option>
        </select>
      }
    >
      {(() => { ensureDefault(); return null; })()}
      <select
        value={selectedModel() ?? ''}
        onChange={(e) => setSelectedModel(e.currentTarget.value)}
        class="h-7 w-48 rounded-md border border-gray-700 bg-gray-800 px-2 text-xs text-gray-200 focus:outline-none focus:ring-1 focus:ring-cyan-500/40"
      >
        <For each={options()}>
          {(opt) => <option value={opt.value}>{opt.label}</option>}
        </For>
      </select>
    </Show>
  );
};

export default ModelSelector;
