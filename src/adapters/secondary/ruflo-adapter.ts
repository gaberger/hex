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

// ─── Typed Errors ────────────────────────────────────────

export class SwarmConnectionError extends Error {
  constructor(
    message: string,
    readonly command: string[],
    readonly cause?: Error,
  ) {
    super(message);
    this.name = 'SwarmConnectionError';
  }
}

export class SwarmParseError extends Error {
  constructor(
    message: string,
    readonly rawOutput: string,
    readonly cause?: Error,
  ) {
    super(message);
    this.name = 'SwarmParseError';
  }
}

const CLI_BIN = 'npx';
const CLI_PKG = '@claude-flow/cli@latest';

export class RufloAdapter implements ISwarmPort {
  constructor(private readonly projectPath: string) {}

  async init(config: SwarmConfig): Promise<SwarmStatus> {
    const result = await this.mcpExec('swarm_init', {
      topology: config.topology,
      maxAgents: config.maxAgents,
      strategy: config.strategy,
    });
    return this.toSwarmStatus(result);
  }

  async status(): Promise<SwarmStatus> {
    const result = await this.mcpExec('swarm_status');
    return this.toSwarmStatus(result);
  }

  async shutdown(): Promise<void> {
    await this.mcpExec('swarm_shutdown');
  }

  async createTask(task: Omit<SwarmTask, 'id' | 'status'>): Promise<SwarmTask> {
    const result = await this.mcpExec('task_create', {
      title: task.title,
      assignee: task.assignee ?? 'unassigned',
      type: task.agentRole,
      ...(task.adapter ? { metadata: `adapter=${task.adapter}` } : {}),
    });
    const id = result.id ?? result.taskId ?? `hex-${Date.now()}`;
    return { ...task, id, status: 'pending' };
  }

  async completeTask(taskId: string, result: string, commitHash?: string): Promise<void> {
    const resultStr = commitHash ? `${result} — commit ${commitHash}` : result;
    await this.mcpExec('task_complete', { id: taskId, result: resultStr });
  }

  async listTasks(statusFilter?: SwarmTask['status']): Promise<SwarmTask[]> {
    const params: Record<string, string> = {};
    if (statusFilter) params.status = statusFilter;
    const result = await this.mcpExec('task_list', params);
    return (result.tasks ?? []) as SwarmTask[];
  }

  async spawnAgent(name: string, role: AgentRole, taskId?: string): Promise<SwarmAgent> {
    const result = await this.mcpExec('agent_spawn', {
      type: role,
      name,
      ...(taskId ? { task: taskId } : {}),
    });
    const id = result.id ?? result.agentId ?? `hex-${Date.now()}`;
    return { id, name, role, status: 'spawning', currentTask: taskId };
  }

  async terminateAgent(agentId: string): Promise<void> {
    await this.mcpExec('agent_terminate', { id: agentId });
  }

  async listAgents(): Promise<SwarmAgent[]> {
    const result = await this.mcpExec('agent_list');
    return (result.agents ?? []) as SwarmAgent[];
  }

  async memoryStore(entry: SwarmMemoryEntry): Promise<void> {
    await this.mcpExec('memory_store', {
      key: entry.key,
      value: entry.value,
      namespace: entry.namespace,
      ...(entry.tags?.length ? { tags: entry.tags.join(',') } : {}),
      ...(entry.ttl ? { ttl: String(entry.ttl) } : {}),
    });
  }

  async memoryRetrieve(key: string, namespace: string): Promise<string | null> {
    try {
      const result = await this.mcpExec('memory_retrieve', { key, namespace });
      return result.value ?? null;
    } catch {
      return null;
    }
  }

  async memorySearch(query: string, namespace?: string): Promise<SwarmMemoryEntry[]> {
    try {
      const result = await this.mcpExec('memory_search', {
        query,
        ...(namespace ? { namespace } : {}),
      });
      return (result.results ?? []) as SwarmMemoryEntry[];
    } catch {
      return [];
    }
  }

  // ─── Private Helpers ─────────────────────────────────────

  /**
   * Execute an MCP tool via the running ruflo daemon.
   * Uses `mcp exec --tool <name>` which routes through the active
   * MCP server process — same state that Claude Code's MCP tools see.
   */
  private async mcpExec(tool: string, params?: Record<string, unknown>): Promise<Record<string, unknown>> {
    const args = ['mcp', 'exec', '--tool', tool];
    if (params) {
      for (const [k, v] of Object.entries(params)) {
        args.push('--param', `${k}=${String(v)}`);
      }
    }

    try {
      const { stdout } = await execFile(CLI_BIN, [CLI_PKG, ...args], {
        cwd: this.projectPath,
        timeout: 30000,
      });
      return this.extractJson(stdout);
    } catch (err) {
      throw new SwarmConnectionError(
        `MCP exec failed: ${tool}`,
        [tool, ...Object.keys(params ?? {})],
        err instanceof Error ? err : undefined,
      );
    }
  }

  /**
   * Extract JSON from `mcp exec` output.
   * Output format is: "Result:\n{...}" — we need just the JSON part.
   */
  private extractJson(output: string): Record<string, unknown> {
    const jsonStart = output.indexOf('{');
    if (jsonStart === -1) return {};
    const jsonStr = output.slice(jsonStart);
    try {
      return JSON.parse(jsonStr) as Record<string, unknown>;
    } catch (err) {
      throw new SwarmParseError(
        'Failed to parse MCP exec response as JSON',
        output,
        err instanceof Error ? err : undefined,
      );
    }
  }

  private toSwarmStatus(result: Record<string, unknown>): SwarmStatus {
    return {
      id: (result.id ?? result.swarmId ?? 'default') as string,
      topology: (result.topology ?? 'hierarchical') as SwarmStatus['topology'],
      agentCount: (result.agentCount ?? 0) as number,
      activeTaskCount: (result.activeTaskCount ?? result.taskCount ?? 0) as number,
      completedTaskCount: (result.completedTaskCount ?? 0) as number,
      status: (result.status ?? 'idle') as SwarmStatus['status'],
    };
  }
}
