/**
 * Ruflo Swarm Adapter
 *
 * Implements ISwarmPort by delegating to @claude-flow/cli.
 * This is the opinionated core of hex-intf — ruflo is required,
 * not optional. The adapter uses execFile (not exec) for safe
 * CLI invocation without shell injection risk.
 */

import { execFile as execFileCb } from 'child_process';
import { promisify } from 'util';
import type {
  ISwarmPort,
  SwarmConfig,
  SwarmStatus,
  SwarmTask,
  SwarmAgent,
  SwarmMemoryEntry,
  AgentRole,
} from '../../core/ports/swarm.js';

const execFile = promisify(execFileCb);

const CLI_BIN = 'npx';
const CLI_PKG = '@claude-flow/cli@latest';

export class RufloAdapter implements ISwarmPort {
  constructor(private readonly projectPath: string) {}

  async init(config: SwarmConfig): Promise<SwarmStatus> {
    const { stdout } = await this.run([
      'swarm', 'init',
      '--topology', config.topology,
      '--max-agents', String(config.maxAgents),
      '--strategy', config.strategy,
    ]);
    return this.parseStatus(stdout);
  }

  async status(): Promise<SwarmStatus> {
    const { stdout } = await this.run(['swarm', 'status', '--json']);
    return this.parseStatus(stdout);
  }

  async shutdown(): Promise<void> {
    await this.run(['swarm', 'shutdown']);
  }

  async createTask(task: Omit<SwarmTask, 'id' | 'status'>): Promise<SwarmTask> {
    const args = ['task', 'create', '--title', task.title, '--assignee', task.assignee ?? 'unassigned'];
    if (task.adapter) args.push('--metadata', `adapter=${task.adapter}`);
    if (task.language) args.push('--metadata', `language=${task.language}`);
    const { stdout } = await this.run(args);
    const id = this.extractId(stdout);
    return { ...task, id, status: 'pending' };
  }

  async completeTask(taskId: string, result: string, commitHash?: string): Promise<void> {
    const resultStr = commitHash ? `${result} — commit ${commitHash}` : result;
    await this.run(['task', 'complete', '--id', taskId, '--result', resultStr]);
  }

  async listTasks(statusFilter?: SwarmTask['status']): Promise<SwarmTask[]> {
    const args = ['task', 'list', '--json'];
    if (statusFilter) args.push('--status', statusFilter);
    const { stdout } = await this.run(args);
    return this.parseTasks(stdout);
  }

  async spawnAgent(name: string, role: AgentRole, taskId?: string): Promise<SwarmAgent> {
    const args = ['agent', 'spawn', '-t', role, '--name', name];
    if (taskId) args.push('--task', taskId);
    const { stdout } = await this.run(args);
    const id = this.extractId(stdout);
    return { id, name, role, status: 'spawning', currentTask: taskId };
  }

  async terminateAgent(agentId: string): Promise<void> {
    await this.run(['agent', 'terminate', '--id', agentId]);
  }

  async listAgents(): Promise<SwarmAgent[]> {
    const { stdout } = await this.run(['agent', 'list', '--json']);
    return this.parseAgents(stdout);
  }

  async memoryStore(entry: SwarmMemoryEntry): Promise<void> {
    const args = ['memory', 'store', '--key', entry.key, '--value', entry.value, '--namespace', entry.namespace];
    if (entry.tags?.length) args.push('--tags', entry.tags.join(','));
    if (entry.ttl) args.push('--ttl', String(entry.ttl));
    await this.run(args);
  }

  async memoryRetrieve(key: string, namespace: string): Promise<string | null> {
    try {
      const { stdout } = await this.run(['memory', 'retrieve', '--key', key, '--namespace', namespace]);
      return stdout.trim() || null;
    } catch {
      return null;
    }
  }

  async memorySearch(query: string, namespace?: string): Promise<SwarmMemoryEntry[]> {
    const args = ['memory', 'search', '--query', query, '--json'];
    if (namespace) args.push('--namespace', namespace);
    const { stdout } = await this.run(args);
    try {
      return JSON.parse(stdout) as SwarmMemoryEntry[];
    } catch {
      return [];
    }
  }

  // ─── Private Helpers ─────────────────────────────────────

  private async run(args: string[]): Promise<{ stdout: string; stderr: string }> {
    return execFile(CLI_BIN, [CLI_PKG, ...args], {
      cwd: this.projectPath,
      timeout: 30000,
    });
  }

  private extractId(output: string): string {
    const match = output.match(/[a-f0-9-]{8,}/);
    return match?.[0] ?? `hex-${Date.now()}`;
  }

  private parseStatus(output: string): SwarmStatus {
    try {
      return JSON.parse(output) as SwarmStatus;
    } catch {
      return {
        id: `swarm-${Date.now()}`,
        topology: 'hierarchical',
        agentCount: 0,
        activeTaskCount: 0,
        completedTaskCount: 0,
        status: 'idle',
      };
    }
  }

  private parseTasks(output: string): SwarmTask[] {
    try { return JSON.parse(output) as SwarmTask[]; } catch { return []; }
  }

  private parseAgents(output: string): SwarmAgent[] {
    try { return JSON.parse(output) as SwarmAgent[]; } catch { return []; }
  }
}
