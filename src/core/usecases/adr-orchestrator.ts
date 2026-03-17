/**
 * ADR Orchestrator use case -- implements IADRQueryPort.
 *
 * Composes IADRPort + ISwarmPort + ICheckpointPort + IWorktreePort
 * to provide ADR lifecycle queries: list, search, abandoned detection,
 * reindexing, and context recall for agent memory injection.
 */

import type { IADRPort } from '../ports/adr.js';
import type { IADRQueryPort } from '../ports/adr.js';
import type { IWorktreePort } from '../ports/index.js';
import type { ADREntry, ADRSnapshot, ADRAbandonedReport, ADRStatus } from '../domain/adr-types.js';

const DEFAULT_STALE_DAYS = 14;
const MS_PER_DAY = 86_400_000;

export class ADROrchestrator implements IADRQueryPort {
  constructor(
    private readonly adr: IADRPort,
    private readonly worktree: IWorktreePort,
  ) {}

  async list(statusFilter?: string): Promise<ADREntry[]> {
    const entries = await this.adr.scanAll();
    if (!statusFilter) return entries;
    const filter = statusFilter.toLowerCase() as ADRStatus;
    return entries.filter((e) => e.status === filter);
  }

  async status(id: string): Promise<ADREntry | null> {
    return this.adr.getById(id);
  }

  async search(query: string, limit = 10): Promise<ADREntry[]> {
    return this.adr.search(query, limit);
  }

  async findAbandoned(staleDays = DEFAULT_STALE_DAYS): Promise<ADRAbandonedReport[]> {
    const entries = await this.adr.scanAll();
    const now = Date.now();
    const reports: ADRAbandonedReport[] = [];

    // Only check proposed ADRs for abandonment
    const proposed = entries.filter((e) => e.status === 'proposed');

    // Get worktree list for cross-referencing
    let worktrees: Array<{ branch: string }> = [];
    try {
      worktrees = await this.worktree.list();
    } catch {
      // Worktree listing failed — proceed without it
    }
    const activeBranches = new Set(worktrees.map((w) => w.branch));

    for (const entry of proposed) {
      const mtime = await this.adr.getLastModified(entry.filePath);
      const daysSinceModified = mtime > 0 ? Math.floor((now - mtime) / MS_PER_DAY) : Infinity;

      let linkedWorktreeStatus: ADRAbandonedReport['linkedWorktreeStatus'] = 'none';
      if (entry.linkedWorktrees.length > 0) {
        const hasActive = entry.linkedWorktrees.some((b) => activeBranches.has(b));
        linkedWorktreeStatus = hasActive ? 'active' : 'stale';
      }

      let recommendation: ADRAbandonedReport['recommendation'] = 'ok';
      if (daysSinceModified >= staleDays) {
        recommendation = linkedWorktreeStatus === 'active' ? 'review' : 'close';
      } else if (daysSinceModified >= staleDays / 2) {
        recommendation = 'review';
      }

      // Only include entries that need attention
      if (recommendation !== 'ok') {
        reports.push({
          adrId: entry.id,
          title: entry.title,
          status: entry.status,
          daysSinceModified: daysSinceModified === Infinity ? -1 : daysSinceModified,
          linkedWorktreeStatus,
          recommendation,
        });
      }
    }

    return reports;
  }

  async reindex(): Promise<{ indexed: number; errors: number }> {
    return this.adr.indexIntoAgentDB();
  }

  async recallForContext(workContext: string): Promise<ADREntry[]> {
    return this.adr.search(workContext, 5);
  }

  async snapshot(): Promise<ADRSnapshot> {
    const entries = await this.adr.scanAll();
    const abandoned = await this.findAbandoned();
    return {
      entries,
      indexedAt: new Date().toISOString(),
      staleProposals: abandoned.map((r) => r.adrId),
    };
  }
}
