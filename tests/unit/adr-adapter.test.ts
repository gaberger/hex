/**
 * ADR Adapter unit tests
 *
 * London-school: mock IFileSystemPort + ISwarmPort, verify markdown parsing
 * and AgentDB pattern store calls.
 */

import { describe, it, expect, mock, beforeEach } from 'bun:test';
import { ADRAdapter, parseADRMarkdown } from '../../src/adapters/secondary/adr-adapter.js';
import type { IFileSystemPort } from '../../src/core/ports/index.js';
import type { ISwarmPort } from '../../src/core/ports/swarm.js';

// ── Sample ADR markdown ─────────────────────────────────

const SAMPLE_ADR = `# ADR-001: Hexagonal Architecture as Foundational Pattern

## Status: Accepted
## Date: 2026-03-15

## Context

AI coding agents need bounded contexts to work effectively.

## Decision

Adopt Hexagonal Architecture (Ports & Adapters).

## Consequences

- Positive: Agents work on one adapter at a time
- Negative: More files and interfaces
`;

const PROPOSED_ADR = `# ADR-099: Proposed Experiment

## Status: Proposed
## Date: 2026-03-01

## Context

This is a proposed experiment.

## Decision

TBD.
`;

// ── Mock factories ──────────────────────────────────────

function mockFS(files: Record<string, string>): IFileSystemPort {
  return {
    read: mock(async (path: string) => {
      if (files[path]) return files[path];
      throw new Error(`File not found: ${path}`);
    }),
    write: mock(async () => {}),
    exists: mock(async (path: string) => path in files),
    glob: mock(async () => Object.keys(files)),
    mtime: mock(async () => Date.now() - 86_400_000 * 5), // 5 days ago
  };
}

function mockSwarm(): ISwarmPort {
  const stored: Array<{ name: string; category: string }> = [];
  return {
    patternStore: mock(async (pattern: any) => {
      stored.push(pattern);
      return { id: `pat-${stored.length}`, ...pattern, accessCount: 0, createdAt: new Date().toISOString(), updatedAt: new Date().toISOString() };
    }),
    patternSearch: mock(async () => []),
    patternFeedback: mock(async () => {}),
    // Stubs for other ISwarmPort methods — not exercised by ADRAdapter
    init: mock(async () => ({ id: 'test', topology: 'hierarchical' as const, agentCount: 0, activeTaskCount: 0, completedTaskCount: 0, status: 'idle' as const })),
    status: mock(async () => ({ id: 'test', topology: 'hierarchical' as const, agentCount: 0, activeTaskCount: 0, completedTaskCount: 0, status: 'idle' as const })),
    shutdown: mock(async () => {}),
    createTask: mock(async () => ({} as any)),
    completeTask: mock(async () => {}),
    listTasks: mock(async () => []),
    spawnAgent: mock(async () => ({} as any)),
    terminateAgent: mock(async () => {}),
    listAgents: mock(async () => []),
    memoryStore: mock(async () => {}),
    memoryRetrieve: mock(async () => null),
    memorySearch: mock(async () => []),
    sessionStart: mock(async () => ({} as any)),
    sessionEnd: mock(async () => {}),
    hierarchicalStore: mock(async () => {}),
    hierarchicalRecall: mock(async () => []),
    consolidate: mock(async () => ({ merged: 0, removed: 0 })),
    contextSynthesize: mock(async () => ''),
    getProgressReport: mock(async () => ({} as any)),
  } as unknown as ISwarmPort;
}

// ── Tests ───────────────────────────────────────────────

describe('parseADRMarkdown', () => {
  it('parses status, date, title, and sections from standard format', () => {
    const entry = parseADRMarkdown(SAMPLE_ADR, 'docs/adrs/ADR-001-hexagonal-architecture.md');
    expect(entry.id).toBe('ADR-001');
    expect(entry.title).toBe('Hexagonal Architecture as Foundational Pattern');
    expect(entry.status).toBe('accepted');
    expect(entry.date).toBe('2026-03-15');
    expect(entry.sections).toContain('Context');
    expect(entry.sections).toContain('Decision');
    expect(entry.sections).toContain('Consequences');
  });

  it('parses proposed status', () => {
    const entry = parseADRMarkdown(PROPOSED_ADR, 'docs/adrs/ADR-099-experiment.md');
    expect(entry.id).toBe('ADR-099');
    expect(entry.status).toBe('proposed');
  });

  it('defaults unrecognized status to proposed', () => {
    const md = '# ADR-050: Test\n\n## Status: Draft\n';
    const entry = parseADRMarkdown(md, 'docs/adrs/ADR-050-test.md');
    expect(entry.status).toBe('proposed');
  });

  it('extracts feature references', () => {
    const md = '# ADR-010: Test\n\n## Status: Accepted\n\nSee feat/adr-tracking and feat/tree-sitter.\n';
    const entry = parseADRMarkdown(md, 'docs/adrs/ADR-010-test.md');
    expect(entry.linkedFeatures).toContain('adr-tracking');
    expect(entry.linkedFeatures).toContain('tree-sitter');
  });
});

describe('ADRAdapter', () => {
  let fs: IFileSystemPort;
  let swarm: ISwarmPort;
  let adapter: ADRAdapter;

  beforeEach(() => {
    fs = mockFS({
      'docs/adrs/ADR-001-hexagonal.md': SAMPLE_ADR,
      'docs/adrs/ADR-099-experiment.md': PROPOSED_ADR,
    });
    swarm = mockSwarm();
    adapter = new ADRAdapter(fs, swarm);
  });

  it('scanAll returns sorted entries', async () => {
    const entries = await adapter.scanAll();
    expect(entries).toHaveLength(2);
    expect(entries[0].id).toBe('ADR-001');
    expect(entries[1].id).toBe('ADR-099');
  });

  it('getById finds existing ADR', async () => {
    const entry = await adapter.getById('ADR-001');
    expect(entry).not.toBeNull();
    expect(entry!.title).toBe('Hexagonal Architecture as Foundational Pattern');
  });

  it('getById returns null for missing ADR', async () => {
    const entry = await adapter.getById('ADR-999');
    expect(entry).toBeNull();
  });

  it('indexIntoAgentDB calls patternStore for each ADR', async () => {
    const result = await adapter.indexIntoAgentDB();
    expect(result.indexed).toBe(2);
    expect(result.errors).toBe(0);
    expect(swarm.patternStore).toHaveBeenCalledTimes(2);
  });

  it('search falls back to local text match when AgentDB returns nothing', async () => {
    const results = await adapter.search('hexagonal');
    expect(results).toHaveLength(1);
    expect(results[0].id).toBe('ADR-001');
  });

  it('search returns empty for no match', async () => {
    const results = await adapter.search('blockchain');
    expect(results).toHaveLength(0);
  });
});
