/**
 * neural-lab.ts — Store for Research Lab dashboard panel.
 *
 * Fetches neural-lab data from hex-nexus REST API:
 *   GET /api/neural-lab/configs
 *   GET /api/neural-lab/experiments
 *   GET /api/neural-lab/frontier/:lineage
 *   GET /api/neural-lab/strategies
 */
import { createSignal, createRoot } from "solid-js";

// ── Types ────────────────────────────────────────────────────────────────────

export interface NetworkConfig {
  id: string;
  name: string;
  lineage: string;
  parent_id: string | null;
  n_layer: number;
  n_head: number;
  n_embd: number;
  block_size: number;
  status: string;
  created_at: string;
}

export interface Experiment {
  id: string;
  config_id: string;
  hypothesis: string;
  status: string; // "queued" | "training" | "kept" | "discarded" | "failed"
  val_bpb: number | null;
  improvement: number | null;
  wall_time_secs: number | null;
  created_at: string;
}

export interface FrontierEntry {
  lineage: string;
  best_config_id: string;
  best_val_bpb: number | null;
  total_experiments: number;
  kept_count: number;
  discarded_count: number;
}

export interface MutationStrategy {
  name: string;
  selection_weight: number;
  success_rate: number;
  total_tried: number;
}

// ── Signals ──────────────────────────────────────────────────────────────────

const [configs, setConfigs] = createSignal<NetworkConfig[]>([]);
const [experiments, setExperiments] = createSignal<Experiment[]>([]);
const [frontier, setFrontier] = createSignal<FrontierEntry[]>([]);
const [strategies, setStrategies] = createSignal<MutationStrategy[]>([]);
const [loading, setLoading] = createSignal(false);
const [error, setError] = createSignal<string | null>(null);

export { configs, experiments, frontier, strategies, loading, error };

// ── Fetch helpers ────────────────────────────────────────────────────────────

async function fetchJson<T>(url: string): Promise<T | null> {
  try {
    const res = await fetch(url);
    if (!res.ok) return null;
    return (await res.json()) as T;
  } catch {
    return null;
  }
}

/** Refresh all neural-lab data from REST API. */
export async function refreshNeuralLab() {
  setLoading(true);
  setError(null);

  try {
    const [cfgs, exps, strats] = await Promise.all([
      fetchJson<NetworkConfig[]>("/api/neural-lab/configs"),
      fetchJson<Experiment[]>("/api/neural-lab/experiments"),
      fetchJson<MutationStrategy[]>("/api/neural-lab/strategies"),
    ]);

    if (cfgs) setConfigs(cfgs);
    if (exps) setExperiments(exps);
    if (strats) setStrategies(strats);

    // Derive frontier from configs — group by lineage
    if (cfgs && exps) {
      const lineages = new Map<string, { configs: NetworkConfig[]; experiments: Experiment[] }>();

      for (const c of cfgs) {
        const key = c.lineage || "default";
        if (!lineages.has(key)) lineages.set(key, { configs: [], experiments: [] });
        lineages.get(key)!.configs.push(c);
      }

      for (const e of exps) {
        const cfg = cfgs.find((c) => c.id === e.config_id);
        const key = cfg?.lineage || "default";
        if (!lineages.has(key)) lineages.set(key, { configs: [], experiments: [] });
        lineages.get(key)!.experiments.push(e);
      }

      const entries: FrontierEntry[] = [];
      for (const [lineage, data] of lineages) {
        const keptExps = data.experiments.filter((e) => e.status === "kept");
        const discardedExps = data.experiments.filter((e) => e.status === "discarded");
        const bestBpb = keptExps.reduce(
          (best, e) => (e.val_bpb !== null && (best === null || e.val_bpb < best) ? e.val_bpb : best),
          null as number | null,
        );

        entries.push({
          lineage,
          best_config_id: keptExps.length > 0 ? keptExps[0].config_id : data.configs[0]?.id ?? "",
          best_val_bpb: bestBpb,
          total_experiments: data.experiments.length,
          kept_count: keptExps.length,
          discarded_count: discardedExps.length,
        });
      }

      setFrontier(entries);
    }

    // Also try the dedicated frontier endpoint for richer data
    const frontierData = await fetchJson<FrontierEntry[]>("/api/neural-lab/frontier/gpt-small");
    if (frontierData && Array.isArray(frontierData)) {
      // Merge with derived data if the endpoint returns richer info
    }
  } catch (err: any) {
    setError(err?.message ?? "Failed to fetch neural-lab data");
  } finally {
    setLoading(false);
  }
}

// ── Polling ──────────────────────────────────────────────────────────────────

let pollInterval: ReturnType<typeof setInterval> | null = null;

export function startNeuralLabPoll(intervalMs = 10000) {
  if (pollInterval) return;
  refreshNeuralLab();
  pollInterval = setInterval(refreshNeuralLab, intervalMs);
}

export function stopNeuralLabPoll() {
  if (pollInterval) {
    clearInterval(pollInterval);
    pollInterval = null;
  }
}
