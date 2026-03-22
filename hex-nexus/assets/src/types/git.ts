/**
 * git.ts — Git domain types (ADR-056).
 * Shared between stores/git.ts and components that display git data.
 */

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

export interface DiffFile {
  path: string;
  status: string;
  additions: number;
  deletions: number;
  patch: string;
}

export interface DiffResult {
  files: DiffFile[];
  totalAdditions: number;
  totalDeletions: number;
  raw: string;
}
