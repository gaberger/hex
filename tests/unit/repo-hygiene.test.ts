import { describe, it, expect } from 'bun:test';
import { analyzeRepoHygiene } from '../../src/core/domain/repo-hygiene.js';
import type { GitStateSnapshot } from '../../src/core/domain/repo-hygiene.js';

describe('analyzeRepoHygiene', () => {
  const clean: GitStateSnapshot = {
    modifiedFiles: [],
    stagedFiles: [],
    untrackedPaths: [],
    worktrees: [],
    embeddedGitDirs: [],
  };

  it('reports clean when no findings', () => {
    const result = analyzeRepoHygiene(clean);
    expect(result.clean).toBe(true);
    expect(result.findings).toHaveLength(0);
  });

  it('detects uncommitted modifications', () => {
    const result = analyzeRepoHygiene({
      ...clean,
      modifiedFiles: ['src/core/domain/foo.ts', 'README.md'],
    });
    expect(result.uncommittedCount).toBe(2);
    expect(result.findings).toHaveLength(2);
    expect(result.findings[0].category).toBe('uncommitted');
    // domain file should be critical severity
    expect(result.findings[0].severity).toBe('critical');
    // README should be info severity
    expect(result.findings[1].severity).toBe('info');
  });

  it('detects staged but uncommitted files', () => {
    const result = analyzeRepoHygiene({
      ...clean,
      stagedFiles: ['src/adapters/primary/cli.ts'],
    });
    expect(result.stagedCount).toBe(1);
    expect(result.findings[0].category).toBe('staged');
    expect(result.findings[0].severity).toBe('warning');
  });

  it('detects orphan worktrees (no recent commits)', () => {
    const result = analyzeRepoHygiene({
      ...clean,
      worktrees: [
        { path: '/repo/wt/stale', branch: 'feat/old', commit: 'abc1234', hasRecentCommits: false },
        { path: '/repo/wt/active', branch: 'feat/new', commit: 'def5678', hasRecentCommits: true },
      ],
    });
    expect(result.orphanWorktreeCount).toBe(1);
    expect(result.findings).toHaveLength(1);
    expect(result.findings[0].category).toBe('orphan-worktree');
    expect(result.findings[0].path).toBe('/repo/wt/stale');
  });

  it('detects embedded git repos as critical', () => {
    const result = analyzeRepoHygiene({
      ...clean,
      embeddedGitDirs: ['examples/weather/node_modules/hex/.git'],
    });
    expect(result.embeddedRepoCount).toBe(1);
    expect(result.findings[0].category).toBe('embedded-repo');
    expect(result.findings[0].severity).toBe('critical');
  });

  it('detects untracked build artifacts', () => {
    const result = analyzeRepoHygiene({
      ...clean,
      untrackedPaths: ['examples/rust-api/target/debug/build'],
    });
    expect(result.findings).toHaveLength(1);
    expect(result.findings[0].category).toBe('build-artifact');
    expect(result.findings[0].severity).toBe('warning');
  });

  it('detects runtime state directories', () => {
    const result = analyzeRepoHygiene({
      ...clean,
      untrackedPaths: ['.hex/status.json', '.superset/config'],
    });
    expect(result.findings).toHaveLength(2);
    expect(result.findings[0].category).toBe('runtime-state');
    expect(result.findings[0].severity).toBe('info');
  });

  it('ignores normal untracked files', () => {
    const result = analyzeRepoHygiene({
      ...clean,
      untrackedPaths: ['src/core/domain/new-feature.ts'],
    });
    // Not a build artifact or runtime state — no finding
    expect(result.findings).toHaveLength(0);
  });

  it('combines all finding types', () => {
    const result = analyzeRepoHygiene({
      modifiedFiles: ['src/core/ports/index.ts'],
      stagedFiles: ['package.json'],
      untrackedPaths: ['.hex/status.json', 'foo/target/bar'],
      worktrees: [{ path: '/old', branch: 'dead', commit: 'aaa', hasRecentCommits: false }],
      embeddedGitDirs: ['vendor/.git'],
    });
    expect(result.clean).toBe(false);
    expect(result.uncommittedCount).toBe(1);
    expect(result.stagedCount).toBe(1);
    expect(result.orphanWorktreeCount).toBe(1);
    expect(result.embeddedRepoCount).toBe(1);
    // 1 uncommitted + 1 staged + 1 orphan + 1 embedded + 1 runtime + 1 build = 6
    expect(result.findings).toHaveLength(6);
  });
});
