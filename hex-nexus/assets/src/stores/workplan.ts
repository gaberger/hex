/**
 * workplan.ts — Workplan data store for hex-nexus dashboard.
 *
 * Fetches workplan execution data from the hex-nexus REST API and provides
 * reactive Solid.js signals for the workplan view. Polls every 5s when an
 * active execution is detected.
 *
 * All reactive primitives are created inside initWorkplanStore() which must
 * be called from the App.tsx composition root (ADR-2603231000).
 *
 * Usage:
 *   import { workplans, activeWorkplan, fetchReport } from "../stores/workplan";
 */
import { createSignal, createRoot, type Accessor, type Setter } from "solid-js";
import { restClient } from "../services/rest-client";

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
  status: "pending" | "active" | "paused" | "completed" | "failed" | "cancelled";
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

// ── Signals (assigned inside createRoot by initWorkplanStore) ─────────────

let workplans: Accessor<WorkplanExecution[]> = () => [];
let setWorkplans: Setter<WorkplanExecution[]> = () => {};
let activeWorkplan: Accessor<WorkplanExecution | null> = () => null;
let setActiveWorkplan: Setter<WorkplanExecution | null> = () => {};
let workplanLoading: Accessor<boolean> = () => false;
let setWorkplanLoading: Setter<boolean> = () => {};
let workplanError: Accessor<string | null> = () => null;
let setWorkplanError: Setter<string | null> = () => {};

export { workplans, activeWorkplan, workplanLoading, workplanError };

// ── Initialization (call from App.tsx composition root) ──────────────────

let _initialized = false;

export function initWorkplanStore() {
  if (_initialized) return;
  _initialized = true;

  createRoot(() => {
    const [_workplans, _setWorkplans] = createSignal<WorkplanExecution[]>([]);
    const [_activeWorkplan, _setActiveWorkplan] = createSignal<WorkplanExecution | null>(null);
    const [_workplanLoading, _setWorkplanLoading] = createSignal(false);
    const [_workplanError, _setWorkplanError] = createSignal<string | null>(null);

    workplans = _workplans;
    setWorkplans = _setWorkplans;
    activeWorkplan = _activeWorkplan;
    setActiveWorkplan = _setActiveWorkplan;
    workplanLoading = _workplanLoading;
    setWorkplanLoading = _setWorkplanLoading;
    workplanError = _workplanError;
    setWorkplanError = _setWorkplanError;
  });
}

// ── Fetchers ──────────────────────────────────────────

let _listInFlight = false;

export async function fetchWorkplans(): Promise<WorkplanExecution[]> {
  if (_listInFlight) return workplans();
  _listInFlight = true;
  setWorkplanLoading(true);
  try {
    const json = await restClient.get<any>("/api/workplan/list");
    const list: WorkplanExecution[] = json.ok ? json.data : (Array.isArray(json) ? json : []);
    setWorkplans(list);
    setWorkplanError(null);

    // Detect active execution (including paused — still needs monitoring)
    const active = list.find(
      (w) => w.status === "active" || w.status === "pending" || w.status === "paused"
    );
    setActiveWorkplan(active ?? null);

    return list;
  } catch (e: any) {
    // Handle 404 (no workplans yet) gracefully
    if (e.message?.includes("404")) {
      setWorkplans([]);
      setActiveWorkplan(null);
      setWorkplanError(null);
      return [];
    }
    setWorkplanError(`Failed to fetch workplans`);
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
    const json = await restClient.get<any>(`/api/workplan/${encodeURIComponent(id)}/report`);
    return json.ok ? json.data : json;
  } catch {
    // Silently fail — caller can handle null
  } finally {
    _reportInFlight = false;
  }
  return null;
}

// ── Actions ──────────────────────────────────────────

export async function executeWorkplan(path: string): Promise<{ ok: boolean; error?: string }> {
  try {
    await restClient.post("/api/workplan/execute", { path });
    await fetchWorkplans();
    return { ok: true };
  } catch (e: any) {
    return { ok: false, error: e.message || "Network error" };
  }
}

export async function pauseWorkplan(): Promise<{ ok: boolean; error?: string }> {
  try {
    await restClient.post("/api/workplan/pause");
    await fetchWorkplans();
    return { ok: true };
  } catch (e: any) {
    return { ok: false, error: e.message || "Network error" };
  }
}

export async function resumeWorkplan(): Promise<{ ok: boolean; error?: string }> {
  try {
    await restClient.post("/api/workplan/resume");
    await fetchWorkplans();
    return { ok: true };
  } catch (e: any) {
    return { ok: false, error: e.message || "Network error" };
  }
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
