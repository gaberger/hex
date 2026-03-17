/**
 * ADR Domain Value Objects
 *
 * Types for Architecture Decision Records lifecycle tracking.
 * ADR markdown files remain the source of truth — AgentDB is the search index.
 *
 * Dependency: pure domain types, no external imports.
 */

// ─── ADR Status ─────────────────────────────────────────

export type ADRStatus = 'proposed' | 'accepted' | 'deprecated' | 'superseded' | 'rejected';

// ─── ADR Entry ──────────────────────────────────────────

/** Parsed representation of a single ADR markdown file. */
export interface ADREntry {
  /** ADR identifier, e.g. "ADR-001" */
  id: string;
  title: string;
  status: ADRStatus;
  /** Date string from the ADR (ISO or freeform). */
  date: string;
  /** Relative file path from project root. */
  filePath: string;
  /** Section headings found in the ADR. */
  sections: string[];
  /** Feature names or IDs this ADR is linked to (extracted from content). */
  linkedFeatures: string[];
  /** Worktree branches associated with this ADR's work. */
  linkedWorktrees: string[];
}

// ─── ADR Snapshot ───────────────────────────────────────

/** Point-in-time snapshot of all ADRs for checkpoint inclusion. */
export interface ADRSnapshot {
  entries: ADREntry[];
  indexedAt: string; // ISO 8601
  staleProposals: string[]; // ADR IDs that are proposed but stale
}

// ─── ADR Abandoned Report ───────────────────────────────

/** Report on a potentially abandoned ADR. */
export interface ADRAbandonedReport {
  adrId: string;
  title: string;
  status: ADRStatus;
  daysSinceModified: number;
  linkedWorktreeStatus: 'active' | 'stale' | 'missing' | 'none';
  recommendation: 'review' | 'accept' | 'close' | 'ok';
}
