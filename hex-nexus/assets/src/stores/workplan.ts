/**
 * workplan.ts — Workplan data store for hex-nexus dashboard.
 *
 * Fetches workplan execution data from the hex-nexus REST API and provides
 * reactive Solid.js signals for the workplan view. Polls every 5s when an
 * active execution is detected.
 *
 * Usage:
 *   import { workplans, activeWorkplan, fetchReport } from "../stores/workplan";
 */
import { createSignal, createEffect, onCleanup } from "solid-js";

// ── Types ─────────────────────────────────────────────

export interface WorkplanPhase {
  name: string;
  status: "pending" | "active" | "passed" | "failed" | "skipped";
  startedAt: string | null;
  completedAt: string | null;
  gateResult: string | null;
  agents: string[];
  commits: string[];
}

export interface WorkplanExecution {
  id: string;
  feature: string;
  status: "pending" | "active" | "completed" | "failed" | "cancelled";
  topology: string;
  createdAt: string;
  startedAt: string | null;
  completedAt: string | null;
  phases: WorkplanPhase[];
  agents: string[];
  currentPhase: string | null;
}

export interface WorkplanReport {
  id: string;
  execution: WorkplanExecution;
  summary: string;
  gateResults: { phase: string; result: string; details: string }[];
  commits: { sha: string; message: string; phase: string; author: string }[];
}

// ── Signals ───────────────────────────────────────────

const [workplans, setWorkplans] = createSignal<WorkplanExecution[]>([]);
const [activeWorkplan, setActiveWorkplan] = createSignal<WorkplanExecution | null>(null);
const [workplanLoading, setWorkplanLoading] = createSignal(false);
const [workplanError, setWorkplanError] = createSignal<string | null>(null);

export { workplans, activeWorkplan, workplanLoading, workplanError };

// ── Fetchers ──────────────────────────────────────────

let _listInFlight = false;

export async function fetchWorkplans(): Promise<WorkplanExecution[]> {
  if (_listInFlight) return workplans();
  _listInFlight = true;
  setWorkplanLoading(true);
  try {
    const res = await fetch("/api/workplan/list");
    if (res.ok) {
      const json = await res.json();
      const list: WorkplanExecution[] = json.ok ? json.data : (Array.isArray(json) ? json : []);
      setWorkplans(list);
      setWorkplanError(null);

      // Detect active execution
      const active = list.find(
        (w) => w.status === "active" || w.status === "pending"
      );
      setActiveWorkplan(active ?? null);

      return list;
    }
    // Non-ok: set empty but don't error for 404 (no workplans yet)
    if (res.status === 404) {
      setWorkplans([]);
      setActiveWorkplan(null);
      setWorkplanError(null);
      return [];
    }
    setWorkplanError(`Failed to fetch workplans: ${res.status}`);
  } catch (e) {
    setWorkplanError(`Network error fetching workplans`);
  } finally {
    _listInFlight = false;
    setWorkplanLoading(false);
  }
  return workplans();
}

let _reportInFlight = false;

export async function fetchReport(id: string): Promise<WorkplanReport | null> {
  if (_reportInFlight) return null;
  _reportInFlight = true;
  try {
    const res = await fetch(`/api/workplan/${encodeURIComponent(id)}/report`);
    if (res.ok) {
      const json = await res.json();
      return json.ok ? json.data : json;
    }
  } catch {
    // Silently fail — caller can handle null
  } finally {
    _reportInFlight = false;
  }
  return null;
}

// ── Polling ───────────────────────────────────────────

let _pollTimer: ReturnType<typeof setInterval> | null = null;
const POLL_INTERVAL = 5_000; // 5 seconds

/**
 * Start polling for workplan updates. Polls every 5s when an active
 * workplan is detected, stops when all executions are terminal.
 */
export function startWorkplanPoll() {
  stopWorkplanPoll();
  // Initial fetch
  fetchWorkplans();

  _pollTimer = setInterval(() => {
    // Only poll if there's an active workplan or we haven't loaded yet
    const active = activeWorkplan();
    if (active || workplans().length === 0) {
      fetchWorkplans();
    }
  }, POLL_INTERVAL);
}

export function stopWorkplanPoll() {
  if (_pollTimer) {
    clearInterval(_pollTimer);
    _pollTimer = null;
  }
}
