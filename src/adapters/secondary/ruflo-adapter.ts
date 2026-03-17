/**
 * Ruflo Swarm Adapter
 *
 * Implements ISwarmPort by delegating to @claude-flow/cli.
 * This is the opinionated core of hex — ruflo is required,
 * not optional. The adapter uses execFile (not exec) for safe
 * CLI invocation without shell injection risk.
 */

import { createRequire } from 'node:module';
import { promisify } from 'node:util';
import { existsSync } from 'node:fs';
import { join, dirname } from 'node:path';
const _require = createRequire(import.meta.url);
const { execFile: execFileCb } = _require('node:child_process');
import type {
  ISwarmPort,
  SwarmConfig,
  SwarmStatus,
  SwarmTask,
  SwarmAgent,
  SwarmMemoryEntry,
  AgentRole,
  AgentDBPattern,
  AgentDBFeedback,
  AgentDBSession,
  AgentDBProgressReport,
} from '../../core/ports/swarm.js';

const execFile = promisify(execFileCb);

// ─── Typed Errors ────────────────────────────────────────

class SwarmConnectionError extends Error {
  constructor(
    message: string,
    readonly command: string[],
    readonly cause?: Error,
  ) {
    super(message);
    this.name = 'SwarmConnectionError';
  }
}

class SwarmTimeoutError extends SwarmConnectionError {
  constructor(tool: string, timeoutMs: number) {
    super(
      `Ruflo command timed out after ${timeoutMs}ms: ${tool}`,
      [tool],
    );
    this.name = 'SwarmTimeoutError';
  }
}

class SwarmParseError extends Error {
  constructor(
    message: string,
    readonly rawOutput: string,
    readonly cause?: Error,
  ) {
    super(message);
    this.name = 'SwarmParseError';
  }
}

// ─── CLI Binary Resolution ──────────────────────────────

const RUFLO_TIMEOUT = 30_000; // 30 seconds default
const HEALTH_CHECK_TTL = 60_000; // cache health check for 60 seconds

/** Cached resolved binary path — resolved once per process. */
let _resolvedBin: { bin: string; args: string[] } | null = null;

/**
 * Resolve the claude-flow CLI binary, preferring a local installation
 * over npx to avoid fetching fresh on every invocation.
 */
function resolveCliBin(projectPath: string): { bin: string; args: string[] } {
  if (_resolvedBin) return _resolvedBin;

  // 1. Check for local node_modules binary
  const localBin = join(projectPath, 'node_modules', '.bin', 'claude-flow');
  if (existsSync(localBin)) {
    _resolvedBin = { bin: localBin, args: [] };
    return _resolvedBin;
  }

  // 2. Try require.resolve to find the package entry, then derive the bin
  try {
    const pkgMain = _require.resolve('@claude-flow/cli');
    // Walk up from the resolved module to find the package's bin directory
    const pkgDir = dirname(dirname(pkgMain)); // e.g. .../node_modules/@claude-flow/cli
    const binPath = join(pkgDir, 'bin', 'claude-flow');
    if (existsSync(binPath)) {
      _resolvedBin = { bin: binPath, args: [] };
      return _resolvedBin;
    }
  } catch {
    // Package not installed locally — fall through to npx
  }

  // 3. Fall back to npx (without @latest to use cached version)
  _resolvedBin = { bin: 'npx', args: ['@claude-flow/cli'] };
  return _resolvedBin;
}

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
  private _healthCache: { healthy: boolean; checkedAt: number } | null = null;

  constructor(
    private readonly projectPath: string,
    private readonly timeoutMs: number = RUFLO_TIMEOUT,
  ) {}

  /**
   * Check if the ruflo daemon is reachable.
   * Result is cached for HEALTH_CHECK_TTL (60s) to avoid repeated checks.
   */
  async healthCheck(): Promise<boolean> {
    const now = Date.now();
    if (this._healthCache && (now - this._healthCache.checkedAt) < HEALTH_CHECK_TTL) {
      return this._healthCache.healthy;
    }

    try {
      await this.mcpExec('task_list');
      this._healthCache = { healthy: true, checkedAt: now };
      return true;
    } catch {
      this._healthCache = { healthy: false, checkedAt: now };
      return false;
    }
  }

  /** Clear the cached health check result (useful after reconnection). */
  clearHealthCache(): void {
    this._healthCache = null;
  }

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
    const id = String(result.id ?? result.taskId ?? `hex-${Date.now()}`);
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
    const id = String(result.id ?? result.agentId ?? `hex-${Date.now()}`);
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
      return (result.value as string) ?? null;
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

  // ─── AgentDB: Pattern Learning ─────────────────────────

  async patternStore(
    pattern: Omit<AgentDBPattern, 'id' | 'accessCount' | 'createdAt' | 'updatedAt'>,
  ): Promise<AgentDBPattern> {
    const result = await this.mcpExec('agentdb_pattern-store', {
      name: pattern.name,
      category: pattern.category,
      content: pattern.content,
      confidence: String(pattern.confidence),
      ...(pattern.tags?.length ? { tags: pattern.tags.join(',') } : {}),
    });
    return {
      id: (result.id as string) ?? `pat-${Date.now()}`,
      name: pattern.name,
      category: pattern.category,
      content: pattern.content,
      confidence: pattern.confidence,
      accessCount: 0,
      createdAt: new Date().toISOString(),
      updatedAt: new Date().toISOString(),
      tags: pattern.tags,
    };
  }

  async patternSearch(query: string, category?: string, limit?: number): Promise<AgentDBPattern[]> {
    try {
      const result = await this.mcpExec('agentdb_pattern-search', {
        query,
        ...(category ? { category } : {}),
        ...(limit ? { limit: String(limit) } : {}),
      });
      return (result.patterns ?? result.results ?? []) as AgentDBPattern[];
    } catch {
      return [];
    }
  }

  async patternFeedback(feedback: AgentDBFeedback): Promise<void> {
    await this.mcpExec('agentdb_feedback', {
      patternId: feedback.patternId,
      outcome: feedback.outcome,
      score: String(feedback.score),
      ...(feedback.context ? { context: feedback.context } : {}),
      ...(feedback.details ? { details: feedback.details } : {}),
    });
  }

  // ─── AgentDB: Session Tracking ─────────────────────────

  async sessionStart(agentName: string, metadata?: Record<string, unknown>): Promise<AgentDBSession> {
    const result = await this.mcpExec('agentdb_session-start', {
      agent: agentName,
      ...(metadata ? { metadata: JSON.stringify(metadata) } : {}),
    });
    return {
      sessionId: (result.sessionId as string) ?? (result.id as string) ?? `sess-${Date.now()}`,
      agentName,
      startedAt: new Date().toISOString(),
      status: 'active',
      metadata,
    };
  }

  async sessionEnd(sessionId: string): Promise<void> {
    await this.mcpExec('agentdb_session-end', { sessionId });
  }

  // ─── AgentDB: Hierarchical Memory ─────────────────────

  async hierarchicalStore(
    layer: string, namespace: string, key: string, value: string, tags?: string[],
  ): Promise<void> {
    await this.mcpExec('agentdb_hierarchical-store', {
      layer,
      namespace,
      key,
      value,
      ...(tags?.length ? { tags: tags.join(',') } : {}),
    });
  }

  async hierarchicalRecall(
    layer: string, namespace?: string, key?: string,
  ): Promise<SwarmMemoryEntry[]> {
    try {
      const result = await this.mcpExec('agentdb_hierarchical-recall', {
        layer,
        ...(namespace ? { namespace } : {}),
        ...(key ? { key } : {}),
      });
      return (result.entries ?? result.results ?? []) as SwarmMemoryEntry[];
    } catch {
      return [];
    }
  }

  // ─── AgentDB: Intelligence ─────────────────────────────

  async consolidate(): Promise<{ merged: number; removed: number }> {
    try {
      const result = await this.mcpExec('agentdb_consolidate');
      return {
        merged: (result.merged as number) ?? 0,
        removed: (result.removed as number) ?? 0,
      };
    } catch {
      return { merged: 0, removed: 0 };
    }
  }

  async contextSynthesize(query: string, sources?: string[]): Promise<string> {
    try {
      const result = await this.mcpExec('agentdb_context-synthesize', {
        query,
        ...(sources?.length ? { sources: sources.join(',') } : {}),
      });
      return (result.context as string) ?? (result.synthesis as string) ?? '';
    } catch {
      return '';
    }
  }

  // ─── AgentDB: Aggregate Progress ──────────────────────

  async getProgressReport(): Promise<AgentDBProgressReport> {
    const [statusResult, tasks, agents, patterns] = await Promise.all([
      this.status().catch(() => ({
        id: 'default', topology: 'hierarchical' as const, agentCount: 0,
        activeTaskCount: 0, completedTaskCount: 0, status: 'idle' as const,
      })),
      this.listTasks().catch(() => []),
      this.listAgents().catch(() => []),
      this.patternSearch('*', undefined, 100).catch(() => []),
    ]);

    const completed = tasks.filter((t) => t.status === 'completed').length;
    const total = tasks.length || 1;

    return {
      swarmId: statusResult.id,
      tasks,
      agents,
      patterns: {
        total: patterns.length,
        recentlyUsed: patterns.filter((p) => p.accessCount > 0).length,
      },
      sessions: [], // Sessions are ephemeral — list via sessionStart tracking
      overallPercent: Math.round((completed / total) * 100),
      phase: statusResult.status === 'running' ? 'execute' : statusResult.status,
    };
  }

  // ─── Private Helpers ─────────────────────────────────────

  /**
   * Execute an MCP tool via the running ruflo daemon.
   * Uses `mcp exec --tool <name>` which routes through the active
   * MCP server process — same state that Claude Code's MCP tools see.
   */
  private async mcpExec(tool: string, params?: Record<string, unknown>): Promise<Record<string, unknown>> {
    const { bin, args: binPrefixArgs } = resolveCliBin(this.projectPath);
    const args = [...binPrefixArgs, 'mcp', 'exec', '--tool', tool];
    if (params) {
      for (const [k, v] of Object.entries(params)) {
        args.push('--param', `${k}=${String(v)}`);
      }
    }

    const fullCmd = [bin, ...args].join(' ');

    try {
      const { stdout, stderr } = await execFile(bin, args, {
        cwd: this.projectPath,
        timeout: this.timeoutMs,
      });

      // Log stderr if present (don't swallow diagnostic output)
      if (stderr && stderr.trim()) {
        process.stderr.write(`[ruflo:${tool}] ${stderr}`);
      }

      return this.extractJson(stdout);
    } catch (err: unknown) {
      // Detect timeout errors specifically
      const isTimeout = err instanceof Error && 'killed' in err && (err as NodeJS.ErrnoException).killed;
      if (isTimeout) {
        throw new SwarmTimeoutError(tool, this.timeoutMs);
      }

      // Include the full command in the error for debugging
      const cause = err instanceof Error ? err : undefined;
      throw new SwarmConnectionError(
        `MCP exec failed [${fullCmd}]: ${cause?.message ?? String(err)}`,
        [tool, ...Object.keys(params ?? {})],
        cause,
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
