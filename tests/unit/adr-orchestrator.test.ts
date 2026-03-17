/**
 * ADR Orchestrator unit tests
 *
 * London-school: mock IADRPort + IWorktreePort, verify orchestration logic
 * including findAbandoned cross-referencing and snapshot building.
 */

import { describe, it, expect, mock, beforeEach } from 'bun:test';
import { ADROrchestrator } from '../../src/core/usecases/adr-orchestrator.js';
import type { IADRPort } from '../../src/core/ports/adr.js';
import type { IWorktreePort } from '../../src/core/ports/index.js';
import type { ADREntry } from '../../src/core/domain/adr-types.js';

// ── Fixtures ────────────────────────────────────────────

const acceptedADR: ADREntry = {
  id: 'ADR-001',
  title: 'Hexagonal Architecture',
  status: 'accepted',
  date: '2026-03-15',
  filePath: 'docs/adrs/ADR-001-hexagonal.md',
  sections: ['Context', 'Decision', 'Consequences'],
  linkedFeatures: [],
  linkedWorktrees: [],
};

const proposedADR: ADREntry = {
  id: 'ADR-099',
  title: 'Proposed Experiment',
  status: 'proposed',
  date: '2026-03-01',
  filePath: 'docs/adrs/ADR-099-experiment.md',
  sections: ['Context', 'Decision'],
  linkedFeatures: [],
  linkedWorktrees: ['feat/experiment'],
};

const freshProposedADR: ADREntry = {
  id: 'ADR-100',
  title: 'Fresh Proposal',
  status: 'proposed',
  date: '2026-03-16',
  filePath: 'docs/adrs/ADR-100-fresh.md',
  sections: ['Context'],
  linkedFeatures: [],
  linkedWorktrees: [],
};

// ── Mock factories ──────────────────────────────────────

function mockADRPort(entries: ADREntry[], mtimeOffset = 20): IADRPort {
  return {
    scanAll: mock(async () => entries),
    getById: mock(async (id: string) => entries.find((e) => e.id === id) ?? null),
    getLastModified: mock(async () => Date.now() - 86_400_000 * mtimeOffset),
    indexIntoAgentDB: mock(async () => ({ indexed: entries.length, errors: 0 })),
    search: mock(async (query: string) => entries.filter((e) => e.title.toLowerCase().includes(query.toLowerCase()))),
  };
}

function mockWorktreePort(branches: string[] = []): IWorktreePort {
  return {
    create: mock(async () => ({ absolutePath: '/tmp/wt', branch: 'test' })),
    merge: mock(async () => ({ success: true, conflicts: [] })),
    cleanup: mock(async () => {}),
    list: mock(async () => branches.map((b) => ({ absolutePath: `/tmp/${b}`, branch: b }))),
  };
}

// ── Tests ───────────────────────────────────────────────

describe('ADROrchestrator', () => {
  describe('list', () => {
    it('returns all entries without filter', async () => {
      const orch = new ADROrchestrator(mockADRPort([acceptedADR, proposedADR]), mockWorktreePort());
      const results = await orch.list();
      expect(results).toHaveLength(2);
    });

    it('filters by status', async () => {
      const orch = new ADROrchestrator(mockADRPort([acceptedADR, proposedADR]), mockWorktreePort());
      const results = await orch.list('accepted');
      expect(results).toHaveLength(1);
      expect(results[0].id).toBe('ADR-001');
    });
  });

  describe('status', () => {
    it('returns entry by id', async () => {
      const orch = new ADROrchestrator(mockADRPort([acceptedADR]), mockWorktreePort());
      const entry = await orch.status('ADR-001');
      expect(entry).not.toBeNull();
      expect(entry!.title).toBe('Hexagonal Architecture');
    });

    it('returns null for missing id', async () => {
      const orch = new ADROrchestrator(mockADRPort([acceptedADR]), mockWorktreePort());
      const entry = await orch.status('ADR-999');
      expect(entry).toBeNull();
    });
  });

  describe('findAbandoned', () => {
    it('detects stale proposed ADR with stale worktree', async () => {
      // mtime 20 days old, worktree not active
      const orch = new ADROrchestrator(
        mockADRPort([acceptedADR, proposedADR], 20),
        mockWorktreePort([]), // no active worktrees
      );
      const reports = await orch.findAbandoned(14);
      expect(reports).toHaveLength(1);
      expect(reports[0].adrId).toBe('ADR-099');
      expect(reports[0].recommendation).toBe('close');
    });

    it('recommends review when worktree is still active', async () => {
      const orch = new ADROrchestrator(
        mockADRPort([proposedADR], 20),
        mockWorktreePort(['feat/experiment']), // worktree is active
      );
      const reports = await orch.findAbandoned(14);
      expect(reports).toHaveLength(1);
      expect(reports[0].recommendation).toBe('review');
      expect(reports[0].linkedWorktreeStatus).toBe('active');
    });

    it('skips accepted ADRs', async () => {
      const orch = new ADROrchestrator(
        mockADRPort([acceptedADR], 30),
        mockWorktreePort(),
      );
      const reports = await orch.findAbandoned(14);
      expect(reports).toHaveLength(0);
    });

    it('returns ok for fresh proposals within threshold', async () => {
      const orch = new ADROrchestrator(
        mockADRPort([freshProposedADR], 2), // only 2 days old
        mockWorktreePort(),
      );
      const reports = await orch.findAbandoned(14);
      expect(reports).toHaveLength(0); // 'ok' entries are filtered out
    });
  });

  describe('reindex', () => {
    it('delegates to adr port', async () => {
      const adrPort = mockADRPort([acceptedADR]);
      const orch = new ADROrchestrator(adrPort, mockWorktreePort());
      const result = await orch.reindex();
      expect(result.indexed).toBe(1);
      expect(adrPort.indexIntoAgentDB).toHaveBeenCalledTimes(1);
    });
  });

  describe('snapshot', () => {
    it('includes all entries and stale proposals', async () => {
      const orch = new ADROrchestrator(
        mockADRPort([acceptedADR, proposedADR], 20),
        mockWorktreePort(),
      );
      const snap = await orch.snapshot();
      expect(snap.entries).toHaveLength(2);
      expect(snap.staleProposals).toContain('ADR-099');
      expect(snap.indexedAt).toBeDefined();
    });
  });

  describe('recallForContext', () => {
    it('searches by work context string', async () => {
      const orch = new ADROrchestrator(
        mockADRPort([acceptedADR, proposedADR]),
        mockWorktreePort(),
      );
      const results = await orch.recallForContext('hexagonal');
      expect(results).toHaveLength(1);
      expect(results[0].id).toBe('ADR-001');
    });
  });
});
