/**
 * git.ts — Git state store for project-scoped git operations (ADR-044).
 *
 * Provides reactive signals for git status, worktrees, branches, and commit log.
 * Data is fetched from the hex-nexus REST API.
 */
import { createSignal } from "solid-js";

// ── Types ─────────────────────────────────────────────

export interface GitStatus {
  branch: string;
  headSha: string;
  isDetached: boolean;
  dirtyCount: number;
  stagedCount: number;
  untrackedCount: number;
  ahead: number;
  behind: number;
  stashCount: number;
  files: StatusFile[];
}

export interface StatusFile {
  path: string;
  status: string;
  staged: boolean;
}

export interface WorktreeInfo {
  path: string;
  branch: string;
  headSha: string;
  isMain: boolean;
  isBare: boolean;
  commitCount: number | null;
}

export interface CommitInfo {
  sha: string;
  shortSha: string;
  message: string;
  authorName: string;
  authorEmail: string;
  timestamp: number;
  parentCount: number;
}

export interface LogResult {
  commits: CommitInfo[];
  hasMore: boolean;
  nextCursor: string | null;
}

export interface BranchInfo {
  name: string;
  sha: string;
  shortSha: string;
  isRemote: boolean;
  isHead: boolean;
}

// ── Signals ───────────────────────────────────────────

const [gitStatus, setGitStatus] = createSignal<GitStatus | null>(null);
const [gitWorktrees, setGitWorktrees] = createSignal<WorktreeInfo[]>([]);
const [gitLog, setGitLog] = createSignal<LogResult | null>(null);
const [gitBranches, setGitBranches] = createSignal<BranchInfo[]>([]);
const [gitLoading, setGitLoading] = createSignal(false);

export { gitStatus, gitWorktrees, gitLog, gitBranches, gitLoading };

// ── Fetchers ──────────────────────────────────────────

export async function fetchGitStatus(projectId: string): Promise<GitStatus | null> {
  try {
    const res = await fetch(`/api/${projectId}/git/status`);
    if (res.ok) {
      const json = await res.json();
      if (json.ok) {
        setGitStatus(json.data);
        return json.data;
      }
    }
  } catch (e) {
    console.error("[git] status fetch failed:", e);
  }
  return null;
}

export async function fetchGitWorktrees(projectId: string): Promise<WorktreeInfo[]> {
  try {
    const res = await fetch(`/api/${projectId}/git/worktrees`);
    if (res.ok) {
      const json = await res.json();
      if (json.ok) {
        const wts = json.data.worktrees ?? [];
        setGitWorktrees(wts);
        return wts;
      }
    }
  } catch (e) {
    console.error("[git] worktrees fetch failed:", e);
  }
  return [];
}

export async function fetchGitLog(
  projectId: string,
  branch?: string,
  cursor?: string,
  limit?: number
): Promise<LogResult | null> {
  try {
    const params = new URLSearchParams();
    if (branch) params.set("branch", branch);
    if (cursor) params.set("cursor", cursor);
    if (limit) params.set("limit", String(limit));
    const qs = params.toString();

    const res = await fetch(`/api/${projectId}/git/log${qs ? "?" + qs : ""}`);
    if (res.ok) {
      const json = await res.json();
      if (json.ok) {
        setGitLog(json.data);
        return json.data;
      }
    }
  } catch (e) {
    console.error("[git] log fetch failed:", e);
  }
  return null;
}

export async function fetchGitBranches(projectId: string): Promise<BranchInfo[]> {
  try {
    const res = await fetch(`/api/${projectId}/git/branches`);
    if (res.ok) {
      const json = await res.json();
      if (json.ok) {
        const branches = json.data.branches ?? [];
        setGitBranches(branches);
        return branches;
      }
    }
  } catch (e) {
    console.error("[git] branches fetch failed:", e);
  }
  return [];
}

/** Fetch all git data for a project in parallel. */
export async function fetchAllGitData(projectId: string): Promise<void> {
  setGitLoading(true);
  try {
    await Promise.all([
      fetchGitStatus(projectId),
      fetchGitWorktrees(projectId),
      fetchGitLog(projectId, undefined, undefined, 10),
    ]);
  } finally {
    setGitLoading(false);
  }
}

// ── WebSocket listener (Phase 2) ─────────────────────

let gitWs: WebSocket | null = null;
let subscribedProjectId: string | null = null;

/**
 * Subscribe to real-time git events for a project via the /ws endpoint.
 * The backend poller broadcasts changes on topic `project:{id}:git`.
 * Events: status-changed, commit-pushed, branch-switched, worktree-created, worktree-removed
 */
export function subscribeGitEvents(projectId: string): void {
  // Already subscribed to this project
  if (subscribedProjectId === projectId && gitWs?.readyState === WebSocket.OPEN) {
    return;
  }

  // Close previous connection
  unsubscribeGitEvents();

  subscribedProjectId = projectId;
  const proto = location.protocol === "https:" ? "wss:" : "ws:";
  const url = `${proto}//${location.host}/ws`;

  try {
    gitWs = new WebSocket(url);

    gitWs.onmessage = (e) => {
      try {
        const msg = JSON.parse(e.data);
        const expectedTopic = `project:${projectId}:git`;

        if (msg.topic !== expectedTopic) return;

        switch (msg.event) {
          case "status-changed":
          case "branch-switched":
            // Refresh full status from REST (WS payload is a summary)
            fetchGitStatus(projectId);
            break;

          case "commit-pushed":
            // Refresh status + log
            fetchGitStatus(projectId);
            fetchGitLog(projectId, undefined, undefined, 10);
            break;

          case "worktree-created":
          case "worktree-removed":
            fetchGitWorktrees(projectId);
            break;
        }
      } catch {
        // Ignore non-JSON or malformed messages
      }
    };

    gitWs.onclose = () => {
      // Auto-reconnect after 5s if still subscribed
      if (subscribedProjectId === projectId) {
        setTimeout(() => {
          if (subscribedProjectId === projectId) {
            subscribeGitEvents(projectId);
          }
        }, 5000);
      }
    };
  } catch {
    // WebSocket creation failed — git data still works via polling REST
  }
}

/** Disconnect from git WebSocket events. */
export function unsubscribeGitEvents(): void {
  subscribedProjectId = null;
  if (gitWs) {
    gitWs.onclose = null; // Prevent auto-reconnect
    gitWs.close();
    gitWs = null;
  }
}
