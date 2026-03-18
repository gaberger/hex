/**
 * HexFlo Adapter — Native swarm coordination via hex-nexus REST API (ADR-027).
 *
 * Replaces RufloAdapter by calling the hex-hub's HexFlo endpoints instead of
 * shelling out to a Node.js CLI. Zero external dependencies.
 */

import type {
  ISwarmPort,
  SwarmConfig,
  SwarmStatus,
  SwarmTask,
  SwarmAgent,
  SwarmMemoryEntry,
  AgentDBPattern,
  AgentDBFeedback,
  AgentDBSession,
  AgentDBProgressReport,
  AgentRole,
} from '../../core/ports/swarm.js';

export class HexFloAdapter implements ISwarmPort {
  private hubUrl: string;
  private currentSwarmId: string | null = null;
  private healthy: boolean | null = null;
  private healthCheckedAt = 0;

  constructor(private projectPath: string, hubUrl = 'http://localhost:5555') {
    this.hubUrl = hubUrl;
  }

  private async api<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
    const opts: RequestInit = {
      method,
      headers: { 'Content-Type': 'application/json' },
    };
    if (body) opts.body = JSON.stringify(body);
    const resp = await fetch(`${this.hubUrl}${path}`, opts);
    if (!resp.ok) {
      const text = await resp.text().catch(() => 'unknown error');
      throw new Error(`HexFlo API ${method} ${path}: ${resp.status} — ${text}`);
    }
    return resp.json() as Promise<T>;
  }

  async healthCheck(): Promise<boolean> {
    const now = Date.now();
    if (this.healthy !== null && now - this.healthCheckedAt < 60_000) {
      return this.healthy;
    }
    try {
      await fetch(`${this.hubUrl}/api/version`, { signal: AbortSignal.timeout(3000) });
      this.healthy = true;
    } catch {
      this.healthy = false;
    }
    this.healthCheckedAt = now;
    return this.healthy;
  }

  async init(config: SwarmConfig): Promise<SwarmStatus> {
    const data = await this.api<{ id: string; topology: string; status: string }>('POST', '/api/swarms', {
      project_id: this.projectPath,
      name: config.memoryNamespace || 'default',
      topology: config.topology,
    });
    this.currentSwarmId = data.id;
    return {
      id: data.id,
      topology: config.topology,
      agentCount: 0,
      activeTaskCount: 0,
      completedTaskCount: 0,
      status: 'running',
    };
  }

  async status(): Promise<SwarmStatus> {
    if (!this.currentSwarmId) {
      return { id: '', topology: 'hierarchical', agentCount: 0, activeTaskCount: 0, completedTaskCount: 0, status: 'idle' };
    }
    const data = await this.api<{ swarm: { id: string; topology: string; status: string }; tasks: unknown[]; agents: unknown[] }>(
      'GET', `/api/swarms/${this.currentSwarmId}`,
    );
    const tasks = data.tasks || [];
    const completed = tasks.filter((t: any) => t.status === 'completed').length;
    return {
      id: data.swarm.id,
      topology: data.swarm.topology as SwarmStatus['topology'],
      agentCount: (data.agents || []).length,
      activeTaskCount: tasks.length - completed,
      completedTaskCount: completed,
      status: data.swarm.status as SwarmStatus['status'],
    };
  }

  async shutdown(): Promise<void> {
    if (this.currentSwarmId) {
      await this.api('PATCH', `/api/swarms/${this.currentSwarmId}`, { status: 'completed' }).catch(() => {});
      this.currentSwarmId = null;
    }
  }

  async createTask(task: Omit<SwarmTask, 'id' | 'status'>): Promise<SwarmTask> {
    if (!this.currentSwarmId) throw new Error('No active swarm — call init() first');
    const data = await this.api<{ id: string }>('POST', `/api/swarms/${this.currentSwarmId}/tasks`, {
      title: task.title,
    });
    return { ...task, id: data.id, status: 'pending' };
  }

  async completeTask(taskId: string, result: string, _commitHash?: string): Promise<void> {
    await this.api('PATCH', `/api/swarms/tasks/${taskId}`, {
      status: 'completed',
      result,
    });
  }

  async listTasks(_statusFilter?: SwarmTask['status']): Promise<SwarmTask[]> {
    if (!this.currentSwarmId) return [];
    const data = await this.api<{ tasks: any[] }>('GET', `/api/swarms/${this.currentSwarmId}`);
    return (data.tasks || []).map((t: any) => ({
      id: t.id,
      title: t.title,
      agentRole: 'coder' as AgentRole,
      status: t.status,
      result: t.result,
      assignee: t.agent_id,
    }));
  }

  async spawnAgent(name: string, role: AgentRole, _taskId?: string): Promise<SwarmAgent> {
    // Agent spawning is handled by hex-hub's AgentManager, not directly by HexFlo
    return { id: `agent-${Date.now()}`, name, role, status: 'active' };
  }

  async terminateAgent(_agentId: string): Promise<void> {
    // Agent lifecycle managed by hex-hub
  }

  async listAgents(): Promise<SwarmAgent[]> {
    try {
      const data = await this.api<{ agents: any[] }>('GET', '/api/agents');
      return (data.agents || []).map((a: any) => ({
        id: a.id || a.agent_id,
        name: a.name || a.agent_name || 'unknown',
        role: 'coder' as AgentRole,
        status: a.status === 'running' ? 'active' as const : 'idle' as const,
      }));
    } catch {
      return [];
    }
  }

  async memoryStore(entry: SwarmMemoryEntry): Promise<void> {
    await this.api('POST', '/api/hexflo/memory', {
      key: entry.key,
      value: entry.value,
      scope: entry.namespace || 'global',
    });
  }

  async memoryRetrieve(key: string, _namespace: string): Promise<string | null> {
    try {
      const data = await this.api<{ value: string }>('GET', `/api/hexflo/memory/${encodeURIComponent(key)}`);
      return data.value || null;
    } catch {
      return null;
    }
  }

  async memorySearch(query: string, _namespace?: string): Promise<SwarmMemoryEntry[]> {
    try {
      const data = await this.api<{ entries: any[] }>('GET', `/api/hexflo/memory/search?q=${encodeURIComponent(query)}`);
      return (data.entries || []).map((e: any) => ({
        key: e.key,
        value: e.value,
        namespace: e.scope || 'global',
      }));
    } catch {
      return [];
    }
  }

  // ─── AgentDB: Pattern Learning (stored via HexFlo memory) ──

  async patternStore(pattern: Omit<AgentDBPattern, 'id' | 'accessCount' | 'createdAt' | 'updatedAt'>): Promise<AgentDBPattern> {
    const id = `pattern-${Date.now()}`;
    const now = new Date().toISOString();
    await this.memoryStore({
      key: `pattern:${id}`,
      value: JSON.stringify(pattern),
      namespace: 'patterns',
    });
    return { ...pattern, id, accessCount: 0, createdAt: now, updatedAt: now };
  }

  async patternSearch(query: string, _category?: string, _limit?: number): Promise<AgentDBPattern[]> {
    const entries = await this.memorySearch(`pattern:${query}`, 'patterns');
    return entries.map(e => {
      try { return JSON.parse(e.value); } catch { return null; }
    }).filter(Boolean);
  }

  async patternFeedback(_feedback: AgentDBFeedback): Promise<void> {
    // Feedback stored as memory entries for now
  }

  // ─── AgentDB: Session Tracking ──

  async sessionStart(agentName: string, metadata?: Record<string, unknown>): Promise<AgentDBSession> {
    const session: AgentDBSession = {
      sessionId: `session-${Date.now()}`,
      agentName,
      startedAt: new Date().toISOString(),
      status: 'active',
      metadata,
    };
    await this.memoryStore({ key: `session:${session.sessionId}`, value: JSON.stringify(session), namespace: 'sessions' });
    return session;
  }

  async sessionEnd(sessionId: string): Promise<void> {
    await this.api('DELETE', `/api/hexflo/memory/${encodeURIComponent(`session:${sessionId}`)}`).catch(() => {});
  }

  // ─── AgentDB: Hierarchical Memory ──

  async hierarchicalStore(layer: string, namespace: string, key: string, value: string, _tags?: string[]): Promise<void> {
    await this.memoryStore({ key: `${layer}:${namespace}:${key}`, value, namespace: layer });
  }

  async hierarchicalRecall(layer: string, namespace?: string, _key?: string): Promise<SwarmMemoryEntry[]> {
    const query = namespace ? `${layer}:${namespace}` : layer;
    return this.memorySearch(query, layer);
  }

  // ─── AgentDB: Intelligence ──

  async consolidate(): Promise<{ merged: number; removed: number }> {
    return { merged: 0, removed: 0 };
  }

  async contextSynthesize(query: string, _sources?: string[]): Promise<string> {
    const entries = await this.memorySearch(query);
    return entries.map(e => `[${e.key}] ${e.value}`).join('\n');
  }

  // ─── Aggregate Progress ──

  async getProgressReport(): Promise<AgentDBProgressReport> {
    const swarmStatus = await this.status();
    const tasks = await this.listTasks();
    const agents = await this.listAgents();
    const completed = tasks.filter(t => t.status === 'completed').length;
    const total = tasks.length || 1;
    return {
      swarmId: swarmStatus.id,
      tasks,
      agents,
      patterns: { total: 0, recentlyUsed: 0 },
      sessions: [],
      overallPercent: Math.round((completed / total) * 100),
      phase: swarmStatus.status,
    };
  }
}
