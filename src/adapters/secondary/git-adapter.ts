/**
 * Git secondary adapter -- implements IGitPort.
 *
 * Uses execFile (no shell) for safety. All commands run in the configured repo.
 */
import { createRequire } from 'node:module';
const _require = createRequire(import.meta.url);
const { execFile: execFileCb } = _require('node:child_process');
import { promisify } from 'node:util';
import { readdir, stat } from 'node:fs/promises';
import { join } from 'node:path';
import type { IGitPort, GitStatusEntry, GitWorktreeEntry } from '../../core/ports/index.js';

const execFile = promisify(execFileCb);

class GitError extends Error {
  override readonly name = 'GitError';
  constructor(
    message: string,
    public readonly command: string,
    public readonly exitCode: number | null,
    public readonly stderr: string,
  ) {
    super(message);
  }
}

export class GitAdapter implements IGitPort {
  constructor(private readonly repoPath: string) {}

  async commit(message: string): Promise<string> {
    await this.git('commit', '-m', message);
    const { stdout } = await this.git('rev-parse', '--short', 'HEAD');
    return stdout.trim();
  }

  async createBranch(name: string): Promise<void> {
    await this.git('checkout', '-b', name);
  }

  async diff(base: string, head: string): Promise<string> {
    const { stdout } = await this.git('diff', `${base}...${head}`);
    return stdout;
  }

  async currentBranch(): Promise<string> {
    const { stdout } = await this.git('rev-parse', '--abbrev-ref', 'HEAD');
    return stdout.trim();
  }

  async statusEntries(): Promise<GitStatusEntry[]> {
    const { stdout } = await this.git('status', '--porcelain');
    if (!stdout.trim()) return [];
    return stdout.trimEnd().split('\n').map((line) => ({
      code: line.slice(0, 2),
      path: line.slice(3),
    }));
  }

  async worktreeEntries(): Promise<GitWorktreeEntry[]> {
    const { stdout } = await this.git('worktree', 'list', '--porcelain');
    if (!stdout.trim()) return [];

    const entries: GitWorktreeEntry[] = [];
    let current: Partial<GitWorktreeEntry> = {};

    for (const line of stdout.split('\n')) {
      if (line.startsWith('worktree ')) {
        if (current.path) entries.push(current as GitWorktreeEntry);
        current = { path: line.slice(9), hasRecentCommits: false };
      } else if (line.startsWith('HEAD ')) {
        current.commit = line.slice(5, 12); // short hash
      } else if (line.startsWith('branch ')) {
        current.branch = line.slice(7).replace('refs/heads/', '');
      }
    }
    if (current.path) entries.push(current as GitWorktreeEntry);

    // Check staleness: skip the main worktree (first entry)
    for (let i = 1; i < entries.length; i++) {
      const wt = entries[i];
      try {
        // Check if branch has commits in last 24h
        const { stdout: logOut } = await execFile('git', [
          '-C', wt.path, 'log', '--oneline', '--since=24.hours.ago', '-1',
        ], { cwd: this.repoPath, maxBuffer: 1024 * 1024 });
        wt.hasRecentCommits = logOut.trim().length > 0;
      } catch {
        wt.hasRecentCommits = false;
      }
    }

    // Exclude the main worktree from hygiene checks
    return entries.slice(1);
  }

  async findEmbeddedRepos(rootPath: string): Promise<string[]> {
    const embedded: string[] = [];
    await this.walkForGitDirs(rootPath, rootPath, embedded, 0);
    return embedded;
  }

  private async walkForGitDirs(
    dir: string,
    rootPath: string,
    results: string[],
    depth: number,
  ): Promise<void> {
    if (depth > 6) return; // don't recurse too deep
    try {
      const entries = await readdir(dir, { withFileTypes: true });
      for (const entry of entries) {
        if (!entry.isDirectory()) continue;
        const full = join(dir, entry.name);
        // Skip known large dirs
        if (entry.name === 'node_modules' || entry.name === 'target' || entry.name === '.git') {
          // If this is .git and NOT the root, it's an embedded repo
          if (entry.name === '.git' && dir !== rootPath) {
            results.push(full);
          }
          continue;
        }
        await this.walkForGitDirs(full, rootPath, results, depth + 1);
      }
    } catch {
      // Permission denied or similar — skip
    }
  }

  private async git(
    ...args: string[]
  ): Promise<{ stdout: string; stderr: string }> {
    try {
      return await execFile('git', args, {
        cwd: this.repoPath,
        maxBuffer: 10 * 1024 * 1024,
      });
    } catch (err: unknown) {
      const e = err as { message: string; code?: number; stderr?: string };
      throw new GitError(
        `Git command failed: ${e.message}`,
        `git ${args.join(' ')}`,
        e.code ?? null,
        e.stderr ?? '',
      );
    }
  }
}
