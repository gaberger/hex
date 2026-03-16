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

/** Validate that `data` is a non-null object with all `requiredKeys` present. */
function validateShape<T>(data: unknown, requiredKeys: string[]): T {
  if (typeof data !== 'object' || data === null) {
    throw new Error('Expected object, got ' + typeof data);
  }
  for (const key of requiredKeys) {
    if (!(key in data)) {
      throw new Error(`Missing required field: ${key}`);
    }
  }
  return data as T;
}

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
      // Memory key may not exist — return null rather than propagating
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
      // Search may fail if swarm daemon is not running — return empty results
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
      return validateShape<Record<string, unknown>>(JSON.parse(jsonStr), []);
    } catch (err) {
      throw new SwarmParseError(
        'Failed to parse MCP exec response as JSON',
        output,
        err instanceof Error ? err : undefined,
      );
    }
  }

  private toSwarmStatus(result: Record<string, unknown>): SwarmStatus {
    const id = result.id ?? result.swarmId ?? 'default';
    const topology = result.topology ?? 'hierarchical';
    const agentCount = result.agentCount ?? 0;
    const activeTaskCount = result.activeTaskCount ?? result.taskCount ?? 0;
    const completedTaskCount = result.completedTaskCount ?? 0;
    const status = result.status ?? 'idle';

    // Validate types at runtime — external CLI output is not type-safe
    if (typeof id !== 'string') throw new SwarmParseError('SwarmStatus.id must be a string', JSON.stringify(result));
    if (typeof agentCount !== 'number') throw new SwarmParseError('SwarmStatus.agentCount must be a number', JSON.stringify(result));
    if (typeof activeTaskCount !== 'number') throw new SwarmParseError('SwarmStatus.activeTaskCount must be a number', JSON.stringify(result));
    if (typeof completedTaskCount !== 'number') throw new SwarmParseError('SwarmStatus.completedTaskCount must be a number', JSON.stringify(result));

    return {
      id,
      topology: topology as SwarmStatus['topology'],
      agentCount,
      activeTaskCount,
      completedTaskCount,
      status: status as SwarmStatus['status'],
    };
  }
}
