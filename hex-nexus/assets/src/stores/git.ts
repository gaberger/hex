/**
 * git.ts — Git state store for project-scoped git operations (ADR-044).
 *
 * Provides reactive signals for git status, worktrees, branches, and commit log.
 * Data is fetched via restClient (ADR-056) from the hex-nexus REST API.
 * Real-time updates arrive via gitWs WebSocket service.
 *
 * Architecture (ADR-039): SpacetimeDB owns project state. The frontend reads
 * project paths from SpacetimeDB and passes them to nexus REST for git operations.
 */
import { createSignal } from "solid-js";
import { addToast } from "./toast";
import { restClient } from '../services/rest-client';
import { gitWs } from '../services/git-ws';

// ── Types (re-exported from canonical location) ──────

export type {
  GitStatus,
  StatusFile,
  WorktreeInfo,
  CommitInfo,
  LogResult,
  BranchInfo,
  DiffFile,
  DiffResult,
} from '../types/git';

import type {
  GitStatus,
  WorktreeInfo,
  LogResult,
  BranchInfo,
  DiffResult,
} from '../types/git';

// ── Signals ───────────────────────────────────────────

const [gitStatus, setGitStatus] = createSignal<GitStatus | null>(null);
const [gitWorktrees, setGitWorktrees] = createSignal<WorktreeInfo[]>([]);
const [gitLog, setGitLog] = createSignal<LogResult | null>(null);
const [gitBranches, setGitBranches] = createSignal<BranchInfo[]>([]);
const [gitDiff, setGitDiff] = createSignal<DiffResult | null>(null);
const [gitLoading, setGitLoading] = createSignal(false);
const [gitError, setGitError] = createSignal<string | null>(null);

export { gitStatus, gitWorktrees, gitLog, gitBranches, gitDiff, gitLoading, gitError };

// ── Helpers ───────────────────────────────────────────

/** Cache of project IDs that have been registered this session. */
const _registeredProjects = new Set<string>();

/** Build git API URL. Ensures project is registered for filesystem access. */
async function ensureRegistered(projectId: string, projectPath?: string): Promise<void> {
  if (!projectPath) return;
  // Skip if already registered this session
  if (_registeredProjects.has(projectId)) return;
  // Lightweight: POST /api/projects/register is idempotent.
  // This tells nexus "I need filesystem access to this path" — not business logic.
  await restClient.post('/api/projects/register', { rootPath: projectPath, name: projectId }).catch(() => {});
  _registeredProjects.add(projectId);
}

// ── Fetchers ──────────────────────────────────────────

let _statusInFlight = false;

export async function fetchGitStatus(projectId: string, projectPath?: string): Promise<GitStatus | null> {
  if (_statusInFlight) return null;
  _statusInFlight = true;
  try {
    await ensureRegistered(projectId, projectPath);
    const json = await restClient.get(`/api/${projectId}/git/status`);
    if (json.ok) {
      setGitStatus(json.data);
      setGitError(null);
      return json.data;
    }
    setGitError("Git status fetch failed");
  } catch (e: any) {
    const msg = e?.message ?? "Git status fetch failed";
    setGitError(msg);
    addToast("error", `Git error: ${msg}`);
  } finally {
    _statusInFlight = false;
  }
  return null;
}

let _worktreesInFlight = false;

export async function fetchGitWorktrees(projectId: string, projectPath?: string): Promise<WorktreeInfo[]> {
  if (_worktreesInFlight) return [];
  _worktreesInFlight = true;
  try {
    await ensureRegistered(projectId, projectPath);
    const json = await restClient.get(`/api/${projectId}/git/worktrees`);
    if (json.ok) {
      const wts = json.data.worktrees ?? [];
      setGitWorktrees(wts);
      setGitError(null);
      return wts;
    }
    setGitError("Git worktrees fetch failed");
  } catch (e: any) {
    const msg = e?.message ?? "Git worktrees fetch failed";
    setGitError(msg);
  } finally {
    _worktreesInFlight = false;
  }
  return [];
}

let _logInFlight = false;

export async function fetchGitLog(
  projectId: string,
  projectPath?: string,
  branch?: string,
  cursor?: string,
  limit?: number
): Promise<LogResult | null> {
  if (_logInFlight) return null;
  _logInFlight = true;
  try {
    await ensureRegistered(projectId, projectPath);
    const params = new URLSearchParams();
    if (branch) params.set("branch", branch);
    if (cursor) params.set("cursor", cursor);
    if (limit) params.set("limit", String(limit));
    const qs = params.toString();

    const json = await restClient.get(`/api/${projectId}/git/log${qs ? "?" + qs : ""}`);
    if (json.ok) {
      setGitLog(json.data);
      setGitError(null);
      return json.data;
    }
    setGitError("Git log fetch failed");
  } catch (e: any) {
    const msg = e?.message ?? "Git log fetch failed";
    setGitError(msg);
  } finally {
    _logInFlight = false;
  }
  return null;
}

let _branchesInFlight = false;
let _branchesBackoff = 0;
let _branchesBackoffTimer: ReturnType<typeof setTimeout> | null = null;

export async function fetchGitBranches(projectId: string, projectPath?: string): Promise<BranchInfo[]> {
  if (_branchesInFlight || _branchesBackoff > 0) return [];
  _branchesInFlight = true;
  try {
    await ensureRegistered(projectId, projectPath);
    const json = await restClient.get(`/api/${projectId}/git/branches`);
    if (json.ok) {
      const branches = json.data.branches ?? [];
      setGitBranches(branches);
      _branchesBackoff = 0;
      return branches;
    }
    // Non-ok response: backoff
    _branchesBackoff = Math.min((_branchesBackoff || 500) * 2, MAX_BACKOFF);
    if (_branchesBackoffTimer) clearTimeout(_branchesBackoffTimer);
    _branchesBackoffTimer = setTimeout(() => { _branchesBackoff = 0; }, _branchesBackoff);
  } catch (e) {
    console.warn("[git] branches fetch failed (will backoff)");
    _branchesBackoff = Math.min((_branchesBackoff || 500) * 2, MAX_BACKOFF);
    if (_branchesBackoffTimer) clearTimeout(_branchesBackoffTimer);
    _branchesBackoffTimer = setTimeout(() => { _branchesBackoff = 0; }, _branchesBackoff);
  } finally {
    _branchesInFlight = false;
  }
  return [];
}

let _diffInFlight = false;
let _diffBackoff = 0;
let _diffBackoffTimer: ReturnType<typeof setTimeout> | null = null;

export async function fetchGitDiff(
  projectId: string,
  projectPath?: string,
  staged?: boolean,
): Promise<DiffResult | null> {
  if (_diffInFlight || _diffBackoff > 0) return null;
  _diffInFlight = true;
  try {
    await ensureRegistered(projectId, projectPath);
    const params = new URLSearchParams();
    if (staged) params.set("staged", "true");
    const qs = params.toString();

    const json = await restClient.get(`/api/${projectId}/git/diff${qs ? "?" + qs : ""}`);
    if (json.ok) {
      setGitDiff(json.data);
      _diffBackoff = 0;
      return json.data;
    }
    if (typeof json === "string" || json.raw) {
      const raw = typeof json === "string" ? json : json.raw;
      const result: DiffResult = { files: [], totalAdditions: 0, totalDeletions: 0, raw };
      setGitDiff(result);
      _diffBackoff = 0;
      return result;
    }
    _diffBackoff = Math.min((_diffBackoff || 500) * 2, MAX_BACKOFF);
    if (_diffBackoffTimer) clearTimeout(_diffBackoffTimer);
    _diffBackoffTimer = setTimeout(() => { _diffBackoff = 0; }, _diffBackoff);
  } catch {
    _diffBackoff = Math.min((_diffBackoff || 500) * 2, MAX_BACKOFF);
    if (_diffBackoffTimer) clearTimeout(_diffBackoffTimer);
    _diffBackoffTimer = setTimeout(() => { _diffBackoff = 0; }, _diffBackoff);
  } finally {
    _diffInFlight = false;
  }
  return null;
}

export async function fetchGitDiffRange(
  projectId: string,
  base: string,
  head: string,
  projectPath?: string,
): Promise<DiffResult | null> {
  try {
    await ensureRegistered(projectId, projectPath);
    const json = await restClient.get(`/api/${projectId}/git/diff/${base}...${head}`);
    if (json.ok) {
      setGitDiff(json.data);
      return json.data;
    }
  } catch (e) {
    console.error("[git] diff-range fetch failed:", e);
  }
  return null;
}

/** Fetch all git data for a project in parallel (with deduplication + backoff). */
let _fetchInFlight = false;
let _fetchBackoff = 0;
let _fetchBackoffTimer: ReturnType<typeof setTimeout> | null = null;
const MAX_BACKOFF = 30_000; // 30s max

export async function fetchAllGitData(projectId: string, projectPath?: string): Promise<void> {
  // Dedup: skip if already fetching
  if (_fetchInFlight) return;
  // Backoff: skip if in cooldown after repeated failures
  if (_fetchBackoff > 0) return;

  _fetchInFlight = true;
  setGitLoading(true);
  try {
    // Ensure registered once before parallel fetches
    await ensureRegistered(projectId, projectPath);
    await Promise.all([
      fetchGitStatus(projectId),
      fetchGitWorktrees(projectId),
      fetchGitLog(projectId, undefined, undefined, undefined, 10),
    ]);
    // Success: reset backoff
    _fetchBackoff = 0;
  } catch {
    // Failure: exponential backoff (1s -> 2s -> 4s -> ... -> 30s)
    _fetchBackoff = Math.min((_fetchBackoff || 500) * 2, MAX_BACKOFF);
    if (_fetchBackoffTimer) clearTimeout(_fetchBackoffTimer);
    _fetchBackoffTimer = setTimeout(() => { _fetchBackoff = 0; }, _fetchBackoff);
  } finally {
    _fetchInFlight = false;
    setGitLoading(false);
  }
}

// ── WebSocket listener (via gitWs service, ADR-056) ──

/** Subscribe to real-time git events for a project. */
export function subscribeGitEvents(projectId: string): void {
  gitWs.subscribe(projectId);
}

/** Unsubscribe from git events and disconnect the WebSocket. */
export function unsubscribeGitEvents(): void {
  gitWs.disconnect();
}

// Wire git WS events to store fetchers
gitWs.onMessage((msg) => {
  const projectId = msg.topic?.split(':')[1];
  if (!projectId) return;
  switch (msg.event) {
    case 'status-changed':
    case 'branch-switched':
      fetchGitStatus(projectId);
      break;
    case 'commit-pushed':
      fetchGitStatus(projectId);
      fetchGitLog(projectId);
      break;
    case 'worktree-created':
    case 'worktree-removed':
      fetchGitWorktrees(projectId);
      break;
  }
});
