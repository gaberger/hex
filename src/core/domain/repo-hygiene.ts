/**
 * Repo Hygiene (Anti-Slop) Analyzer
 *
 * Pure domain function — takes raw git state data and categorizes
 * findings into actionable hygiene items. No I/O.
 */

import type {
  HygieneCategory,
  HygieneFinding,
  HygieneSeverity,
  RepoHygieneResult,
} from './value-objects.js';

// ─── Input: Raw Git State (provided by git port) ──────

export interface GitStateSnapshot {
  /** Files with unstaged modifications (git status: ' M') */
  modifiedFiles: string[];
  /** Files staged but not committed (git status: 'M ', 'A ', etc.) */
  stagedFiles: string[];
  /** Untracked files/dirs (git status: '??') */
  untrackedPaths: string[];
  /** git worktree list output: { path, branch, commit } */
  worktrees: WorktreeInfo[];
  /** Paths containing .git directories (not the root) */
  embeddedGitDirs: string[];
}

interface WorktreeInfo {
  path: string;
  branch: string;
  commit: string;
  /** Whether any commits exist on this branch after diverging from main */
  hasRecentCommits: boolean;
}

// ─── Build Artifact / Runtime State Patterns ───────────

const BUILD_ARTIFACT_PATTERNS = [
  /\/target\//,          // Rust/Cargo
  /\/dist\//,            // JS/TS build output
  /\/build\//,           // generic build dirs
  /\/node_modules\//,    // npm dependencies
  /\/__pycache__\//,     // Python
  /\/\.next\//,          // Next.js
  /\/\.nuxt\//,          // Nuxt
];

const RUNTIME_STATE_PATTERNS = [
  /^\.hex\//,            // hex runtime state
  /^\.superset\//,       // superset state
  /^\.claude\//,         // claude code state (when untracked)
  /\/\.db$/,             // database files
  /\/\.sqlite$/,         // sqlite databases
];

// ─── Core Analysis ─────────────────────────────────────

export function analyzeRepoHygiene(snapshot: GitStateSnapshot): RepoHygieneResult {
  const findings: HygieneFinding[] = [];

  // 1. Uncommitted modifications
  for (const file of snapshot.modifiedFiles) {
    findings.push({
      category: 'uncommitted',
      severity: classifyUncommittedSeverity(file),
      path: file,
      description: `Modified but not staged: ${file}`,
      suggestedFix: `git add ${file}`,
    });
  }

  // 2. Staged but not committed
  for (const file of snapshot.stagedFiles) {
    findings.push({
      category: 'staged',
      severity: 'warning',
      path: file,
      description: `Staged but not committed: ${file}`,
      suggestedFix: 'git commit (or git reset HEAD to unstage)',
    });
  }

  // 3. Orphan worktrees (no recent commits)
  for (const wt of snapshot.worktrees) {
    if (!wt.hasRecentCommits) {
      findings.push({
        category: 'orphan-worktree',
        severity: 'warning',
        path: wt.path,
        description: `Stale worktree on branch ${wt.branch} — no recent commits`,
        suggestedFix: `git worktree remove ${wt.path}`,
      });
    }
  }

  // 4. Embedded git repos
  for (const gitDir of snapshot.embeddedGitDirs) {
    findings.push({
      category: 'embedded-repo',
      severity: 'critical',
      path: gitDir,
      description: `Embedded .git directory — risks accidental submodule or nested repo`,
      suggestedFix: `Remove with rm -rf ${gitDir} or convert to submodule`,
    });
  }

  // 5. Untracked build artifacts
  for (const path of snapshot.untrackedPaths) {
    if (isBuildArtifact(path)) {
      findings.push({
        category: 'build-artifact',
        severity: 'warning',
        path,
        description: `Untracked build artifact should be gitignored: ${path}`,
        suggestedFix: `Add ${extractGitignorePattern(path)} to .gitignore`,
      });
    } else if (isRuntimeState(path)) {
      findings.push({
        category: 'runtime-state',
        severity: 'info',
        path,
        description: `Runtime state directory should be gitignored: ${path}`,
        suggestedFix: `Add ${path} to .gitignore`,
      });
    }
  }

  return {
    findings,
    uncommittedCount: snapshot.modifiedFiles.length,
    stagedCount: snapshot.stagedFiles.length,
    orphanWorktreeCount: snapshot.worktrees.filter((w) => !w.hasRecentCommits).length,
    embeddedRepoCount: snapshot.embeddedGitDirs.length,
    clean: findings.length === 0,
  };
}

// ─── Helpers ───────────────────────────────────────────

function classifyUncommittedSeverity(file: string): HygieneSeverity {
  // Source files in core layers are higher priority
  if (file.includes('/domain/') || file.includes('/ports/')) return 'critical';
  if (file.includes('/usecases/') || file.includes('/adapters/')) return 'warning';
  return 'info';
}

function isBuildArtifact(path: string): boolean {
  return BUILD_ARTIFACT_PATTERNS.some((p) => p.test(path));
}

function isRuntimeState(path: string): boolean {
  return RUNTIME_STATE_PATTERNS.some((p) => p.test(path));
}

function extractGitignorePattern(path: string): string {
  // Extract the directory pattern for gitignore (e.g., "target/" from "foo/target/bar")
  for (const pattern of BUILD_ARTIFACT_PATTERNS) {
    const match = path.match(pattern);
    if (match) {
      const idx = path.indexOf(match[0]);
      return path.slice(0, idx) + match[0];
    }
  }
  return path;
}
