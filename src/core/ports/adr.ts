/**
 * ADR Port Interfaces
 *
 * Contracts for ADR lifecycle tracking with AgentDB integration.
 *
 * IADRPort (secondary/driven): Scans filesystem, parses markdown, indexes into AgentDB.
 * IADRQueryPort (primary/driving): Query interface consumed by CLI and MCP adapters.
 *
 * Dependency: domain/adr-types.ts only.
 */

import type { ADREntry, ADRSnapshot, ADRAbandonedReport } from '../domain/adr-types.js';

// ─── Output Port (Secondary / Driven) ───────────────────

export interface IADRPort {
  /** Scan all ADR markdown files and return parsed entries. */
  scanAll(): Promise<ADREntry[]>;

  /** Get a single ADR by its ID (e.g. "ADR-001"). Returns null if not found. */
  getById(id: string): Promise<ADREntry | null>;

  /** Get the last-modified timestamp (epoch ms) for an ADR file. Returns 0 if unavailable. */
  getLastModified(filePath: string): Promise<number>;

  /** Index all ADRs into AgentDB as searchable patterns. Best-effort (no throw). */
  indexIntoAgentDB(): Promise<{ indexed: number; errors: number }>;

  /** Search ADRs via AgentDB pattern search. Falls back to local text match. */
  search(query: string, limit?: number): Promise<ADREntry[]>;
}

// ─── Input Port (Primary / Driving) ─────────────────────

export interface IADRQueryPort {
  /** List all ADRs with optional status filter. */
  list(statusFilter?: string): Promise<ADREntry[]>;

  /** Get the status of a specific ADR by ID. */
  status(id: string): Promise<ADREntry | null>;

  /** Search ADRs by text query (uses AgentDB when available). */
  search(query: string, limit?: number): Promise<ADREntry[]>;

  /** Find ADRs that appear abandoned (proposed + stale). */
  findAbandoned(staleDays?: number): Promise<ADRAbandonedReport[]>;

  /** Re-index all ADRs into AgentDB. */
  reindex(): Promise<{ indexed: number; errors: number }>;

  /** Recall ADRs relevant to a work context (for agent memory injection). */
  recallForContext(workContext: string): Promise<ADREntry[]>;

  /** Build a snapshot for checkpoint inclusion. */
  snapshot(): Promise<ADRSnapshot>;
}
