/**
 * Worktree secondary adapter -- implements IWorktreePort.
 *
 * Uses `git worktree` commands via execFile (no shell) to manage isolated
 * working directories for parallel branch operations.
 */
import { execFile as execFileCb } from 'node:child_process';
import { join } from 'node:path';
import { promisify } from 'node:util';
import type { IWorktreePort, MergeResult, WorktreePath } from '../../core/ports/index.js';

const execFile = promisify(execFileCb);

export class WorktreeError extends Error {
  override readonly name = 'WorktreeError';
  constructor(message: string, public readonly command: string) {
    super(message);
  }
}

export class WorktreeAdapter implements IWorktreePort {
  constructor(
    private readonly repoPath: string,
    private readonly worktreeDir: string,
  ) {}

  async create(branchName: string): Promise<WorktreePath> {
    const absolutePath = this.worktreePath(branchName);
    await this.git('worktree', 'add', absolutePath, '-b', branchName);
    return { absolutePath, branch: branchName };
  }

  async merge(worktree: WorktreePath, target: string): Promise<MergeResult> {
    await this.git('checkout', target);
    try {
      const { stdout } = await this.git('merge', worktree.branch);
      const hashMatch = stdout.match(/([0-9a-f]{7,40})/);
      return { success: true, conflicts: [], commitHash: hashMatch?.[1] };
    } catch (err: unknown) {
      const e = err as { stderr?: string };
      const conflicts = (e.stderr ?? '')
        .split('\n')
        .filter((l) => l.startsWith('CONFLICT'))
        .map((l) => l.replace(/^CONFLICT[^:]*:\s*/, '').trim());
      return { success: false, conflicts };
    }
  }

  async cleanup(worktree: WorktreePath): Promise<void> {
    await this.git('worktree', 'remove', worktree.absolutePath, '--force');
  }

  async list(): Promise<WorktreePath[]> {
    const { stdout } = await this.git('worktree', 'list', '--porcelain');
    const entries: WorktreePath[] = [];
    let currentPath = '';
    for (const line of stdout.split('\n')) {
      if (line.startsWith('worktree ')) currentPath = line.slice(9);
      else if (line.startsWith('branch refs/heads/') && currentPath) {
        entries.push({ absolutePath: currentPath, branch: line.slice(18) });
      }
    }
    return entries;
  }

  private worktreePath(branchName: string): string {
    return join(this.worktreeDir, `hex-intf-${branchName}`);
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
      const e = err as { message: string };
      throw new WorktreeError(
        `Worktree command failed: ${e.message}`,
        `git ${args.join(' ')}`,
      );
    }
  }
}
