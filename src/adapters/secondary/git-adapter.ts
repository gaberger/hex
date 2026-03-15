/**
 * Git secondary adapter -- implements IGitPort.
 *
 * Uses execFile (no shell) for safety. All commands run in the configured repo.
 */
import { execFile as execFileCb } from 'node:child_process';
import { promisify } from 'node:util';
import type { IGitPort } from '../../core/ports/index.js';

const execFile = promisify(execFileCb);

export class GitError extends Error {
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
