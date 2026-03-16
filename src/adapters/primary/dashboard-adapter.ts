/**
 * Dashboard Client Adapter (Bidirectional)
 *
 * Connects to the central Dashboard Hub (port 5555) as a CLIENT.
 * Two communication channels:
 *
 * 1. HTTP POST (push) — Gathers data from the local project's AppContext
 *    and pushes health, tokens, swarm, and graph data to the hub periodically.
 *
 * 2. WebSocket (command listener) — Opens a persistent WS connection to the
 *    hub, subscribes to project:{id}:command topic, and dispatches received
 *    commands through AppContext. Results are POSTed back to the hub.
 *
 * This makes hex-hub fully bidirectional: monitor status AND issue commands
 * from a single place (browser, MCP, or CLI).
 */

import { watch, readFileSync, type FSWatcher } from 'node:fs';
import { resolve, join } from 'node:path';
import { homedir } from 'node:os';
import { request } from 'node:http';
import WebSocket from 'ws';
import type { AppContext } from '../../core/ports/app-context.js';
import type { ImportEdge } from '../../core/ports/index.js';
import type {
  HubCommand,
  HubCommandHandler,
  HubCommandResult,
  HubCommandType,
  IHubCommandReceiverPort,
} from '../../core/ports/hub-command.js';

/** Fixed dashboard hub port — must match dashboard-hub.ts */
const HUB_PORT = 5555;

/** Read auth token from hub lock file. Returns empty string if unavailable. */
function readHubToken(): string {
  try {
    const lockPath = join(homedir(), '.hex', 'daemon', 'hub.lock');
    const lock = JSON.parse(readFileSync(lockPath, 'utf-8'));
    return lock.token ?? '';
  } catch {
    return '';
  }
}

// ── Layer classifier ─────────────────────────────────────

function classifyLayer(filePath: string): string {
  if (filePath.includes('/core/domain/')) return 'domain';
  if (filePath.includes('/core/ports/')) return 'port';
  if (filePath.includes('/core/usecases/')) return 'usecase';
  if (filePath.includes('/adapters/primary/')) return 'primary-adapter';
  if (filePath.includes('/adapters/secondary/')) return 'secondary-adapter';
  return 'other';
}

// ── Dashboard Client ─────────────────────────────────────

export class DashboardAdapter implements IHubCommandReceiverPort {
  private projectId: string | null = null;
  private pushTimer: ReturnType<typeof setInterval> | null = null;
  private watchers: FSWatcher[] = [];
  private fileChangeDebounce = new Map<string, ReturnType<typeof setTimeout>>();
  private stopped = false;
  private readonly authToken: string;

  // ── WebSocket command listener state ──────────────
  private ws: WebSocket | null = null;
  private wsReconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private wsReconnectDelay = 1000; // starts at 1s, doubles up to 30s
  private readonly wsMaxReconnectDelay = 30_000;
  private commandHandlers = new Map<HubCommandType, HubCommandHandler>();
  private _isListening = false;

  constructor(
    private readonly ctx: AppContext,
    private readonly hubPort: number = HUB_PORT,
  ) {
    this.authToken = readHubToken();
  }

  /**
   * Register with the hub and start pushing data.
   * Returns the hub URL for display purposes.
   * If the hub is not running, throws (caller can catch and skip).
   */
  async start(): Promise<{ url: string; close: () => void }> {
    const hubUrl = `http://localhost:${this.hubPort}`;

    // Register with hub
    const name = this.ctx.rootPath.split('/').pop() ?? 'unknown';
    const regResult = await this.post('/api/projects/register', {
      name,
      rootPath: this.ctx.rootPath,
      astIsStub: this.ctx.astIsStub,
    });

    if (!regResult || !regResult.id) {
      throw new Error('Hub registration failed');
    }

    this.projectId = regResult.id as string;

    // Initial push — fire and forget (don't block registration)
    void this.pushAll().catch((err) => this.log('initial push failed:', err));

    // Start periodic push (every 10s)
    this.pushTimer = setInterval(() => {
      if (!this.stopped) void this.pushAll().catch((err) => this.log('periodic push failed:', err));
    }, 10_000);
    this.pushTimer.unref(); // Don't keep process alive

    // Watch local files for changes
    this.startFileWatcher();

    // Open WebSocket for bidirectional command channel
    void this.startListening().catch((err) => this.log('command listener failed to start:', err));

    return {
      url: hubUrl,
      close: () => this.stop(),
    };
  }

  /** Stop pushing, close command listener, and unregister. */
  stop(): void {
    this.stopped = true;
    if (this.pushTimer) {
      clearInterval(this.pushTimer);
      this.pushTimer = null;
    }
    for (const w of this.watchers) w.close();
    this.watchers = [];
    for (const t of this.fileChangeDebounce.values()) clearTimeout(t);
    this.fileChangeDebounce.clear();
    void this.stopListening();
  }

  // ── IHubCommandReceiverPort implementation ─────────

  async startListening(): Promise<void> {
    if (!this.projectId || this._isListening) return;
    this._isListening = true; // Set immediately to prevent concurrent calls (H2)

    const tokenParam = this.authToken ? `?token=${encodeURIComponent(this.authToken)}` : '';
    const wsUrl = `ws://127.0.0.1:${this.hubPort}/ws${tokenParam}`;

    this.registerDefaultHandlers();
    this.connectWs(wsUrl);
  }

  async stopListening(): Promise<void> {
    this._isListening = false;
    if (this.wsReconnectTimer) {
      clearTimeout(this.wsReconnectTimer);
      this.wsReconnectTimer = null;
    }
    if (this.ws) {
      this.ws.removeAllListeners();
      if (this.ws.readyState === WebSocket.OPEN || this.ws.readyState === WebSocket.CONNECTING) {
        this.ws.close(1000, 'shutdown');
      }
      this.ws = null;
    }
  }

  isListening(): boolean {
    return this._isListening;
  }

  onCommand(type: HubCommandType, handler: HubCommandHandler): void {
    this.commandHandlers.set(type, handler);
  }

  offCommand(type: HubCommandType): void {
    this.commandHandlers.delete(type);
  }

  // ── WebSocket connection with auto-reconnect ──────

  private connectWs(wsUrl: string): void {
    if (this.stopped) return;

    try {
      this.ws = new WebSocket(wsUrl);
    } catch {
      this.scheduleReconnect(wsUrl);
      return;
    }

    this.ws.on('open', () => {
      this._isListening = true;
      this.wsReconnectDelay = 1000; // reset backoff on successful connect
      this.log('command listener connected');

      // Subscribe to command topic for this project
      this.wsSend({
        type: 'subscribe',
        topic: `project:${this.projectId}:command`,
      });
    });

    this.ws.on('message', (raw: WebSocket.Data) => {
      try {
        const envelope = JSON.parse(raw.toString());
        if (envelope.event === 'command' && envelope.data) {
          void this.handleCommand(envelope.data as HubCommand);
        }
      } catch {
        // Ignore malformed messages
      }
    });

    this.ws.on('close', () => {
      this._isListening = false;
      this.scheduleReconnect(wsUrl);
    });

    this.ws.on('error', () => {
      // WS errors are non-critical — HTTP push still works.
      // 'close' event fires after 'error', triggering reconnect silently.
    });
  }

  private scheduleReconnect(wsUrl: string): void {
    if (this.stopped) return; // Check stopped FIRST — prevents reconnect after shutdown (H3)
    if (this.wsReconnectTimer) return;

    this.wsReconnectTimer = setTimeout(() => {
      this.wsReconnectTimer = null;
      this.connectWs(wsUrl);
    }, this.wsReconnectDelay);
    this.wsReconnectTimer.unref();

    // Exponential backoff: 1s → 2s → 4s → 8s → 16s → 30s (cap)
    this.wsReconnectDelay = Math.min(this.wsReconnectDelay * 2, this.wsMaxReconnectDelay);
  }

  private wsSend(msg: unknown): void {
    if (this.ws?.readyState === WebSocket.OPEN) {
      this.ws.send(JSON.stringify(msg));
    }
  }

  // ── Command dispatch ──────────────────────────────

  private async handleCommand(command: HubCommand): Promise<void> {
    const handler = this.commandHandlers.get(command.type);
    let result: HubCommandResult;

    if (!handler) {
      result = {
        commandId: command.commandId,
        status: 'failed',
        error: `Unknown command type: ${command.type}`,
        completedAt: new Date().toISOString(),
      };
    } else {
      try {
        result = await handler(command);
      } catch (err) {
        result = {
          commandId: command.commandId,
          status: 'failed',
          error: err instanceof Error ? err.message : String(err),
          completedAt: new Date().toISOString(),
        };
      }
    }

    // Report result back to hub via HTTP POST (hub broadcasts to WS subscribers)
    await this.post(
      `/api/${this.projectId}/command/${command.commandId}/result`,
      result,
    );
  }

  // ── Default command handlers ──────────────────────

  private registerDefaultHandlers(): void {
    this.onCommand('ping', async (cmd) => ({
      commandId: cmd.commandId,
      status: 'completed',
      data: { pong: true, projectId: this.projectId, timestamp: Date.now() },
      completedAt: new Date().toISOString(),
    }));

    this.onCommand('spawn-agent', async (cmd) => {
      const p = cmd.payload as { name: string; role: string; taskId?: string };
      const agent = await this.ctx.swarm.spawnAgent(
        p.name,
        p.role as import('../../core/ports/swarm.js').AgentRole,
        p.taskId,
      );
      return {
        commandId: cmd.commandId,
        status: 'completed',
        data: agent,
        completedAt: new Date().toISOString(),
      };
    });

    this.onCommand('terminate-agent', async (cmd) => {
      const p = cmd.payload as { agentId: string };
      await this.ctx.swarm.terminateAgent(p.agentId);
      return {
        commandId: cmd.commandId,
        status: 'completed',
        data: { terminated: p.agentId },
        completedAt: new Date().toISOString(),
      };
    });

    this.onCommand('create-task', async (cmd) => {
      const p = cmd.payload as { title: string; agentRole: string; adapter?: string; language?: string };
      const task = await this.ctx.swarm.createTask({
        title: p.title,
        agentRole: p.agentRole as import('../../core/ports/swarm.js').AgentRole,
        adapter: p.adapter,
        language: p.language as import('../../core/ports/swarm.js').SwarmTask['language'],
      });
      return {
        commandId: cmd.commandId,
        status: 'completed',
        data: task,
        completedAt: new Date().toISOString(),
      };
    });

    this.onCommand('cancel-task', async (cmd) => {
      const p = cmd.payload as { taskId: string };
      await this.ctx.swarm.completeTask(p.taskId, 'cancelled');
      return {
        commandId: cmd.commandId,
        status: 'completed',
        data: { cancelled: p.taskId },
        completedAt: new Date().toISOString(),
      };
    });

    this.onCommand('run-analyze', async (cmd) => {
      const p = cmd.payload as { rootPath?: string };
      const result = await this.ctx.archAnalyzer.analyzeArchitecture(p.rootPath ?? '.');
      return {
        commandId: cmd.commandId,
        status: 'completed',
        data: result,
        completedAt: new Date().toISOString(),
      };
    });

    this.onCommand('run-build', async (cmd) => {
      const name = this.ctx.rootPath.split('/').pop() ?? 'unknown';
      const result = await this.ctx.build.compile({ rootPath: this.ctx.rootPath, name, language: 'typescript', adapters: [] });
      return {
        commandId: cmd.commandId,
        status: result.success ? 'completed' : 'failed',
        data: result,
        error: result.success ? undefined : 'Build failed',
        completedAt: new Date().toISOString(),
      };
    });

    this.onCommand('run-validate', async (cmd) => {
      const p = cmd.payload as { rootPath?: string };
      // Validate = analyze + build combined check
      const [analysis, buildResult] = await Promise.all([
        this.ctx.archAnalyzer.analyzeArchitecture(p.rootPath ?? '.'),
        this.ctx.build.compile({
          rootPath: this.ctx.rootPath,
          name: this.ctx.rootPath.split('/').pop() ?? 'unknown',
          language: 'typescript',
          adapters: [],
        }),
      ]);
      const violations = (analysis as { summary?: { violationCount?: number } }).summary?.violationCount ?? 0;
      const passed = buildResult.success && violations === 0;
      return {
        commandId: cmd.commandId,
        status: passed ? 'completed' : 'failed',
        data: { passed, build: buildResult.success, violations, analysis },
        error: passed ? undefined : `Validation failed: build=${buildResult.success}, violations=${violations}`,
        completedAt: new Date().toISOString(),
      };
    });

    this.onCommand('run-generate', async (cmd) => {
      const p = cmd.payload as { adapter?: string; portInterface?: string; language?: string };
      if (!this.ctx.codeGenerator) {
        return {
          commandId: cmd.commandId,
          status: 'failed',
          error: 'Code generation requires an LLM API key (set ANTHROPIC_API_KEY)',
          completedAt: new Date().toISOString(),
        };
      }
      if (!p.adapter || !p.portInterface) {
        return {
          commandId: cmd.commandId,
          status: 'failed',
          error: 'Missing required payload: adapter and portInterface (e.g. run-generate my-adapter IMyPort)',
          completedAt: new Date().toISOString(),
        };
      }
      const lang = (p.language ?? 'typescript') as import('../../core/domain/value-objects.js').Language;
      const spec: import('../../core/domain/value-objects.js').Specification = {
        title: `Generate ${p.adapter} implementing ${p.portInterface}`,
        requirements: [`Implement ${p.portInterface} adapter: ${p.adapter}`],
        constraints: ['Follow hexagonal architecture rules', 'Only import from core/ports and core/domain'],
        targetLanguage: lang,
        targetAdapter: p.adapter,
      };
      const result = await this.ctx.codeGenerator.generateFromSpec(spec, lang);
      return {
        commandId: cmd.commandId,
        status: 'completed',
        data: result,
        completedAt: new Date().toISOString(),
      };
    });

    this.onCommand('run-summarize', async (cmd) => {
      const p = cmd.payload as { filePath?: string; level?: string };
      if (p.filePath) {
        const summary = await this.ctx.ast.extractSummary(
          p.filePath,
          (p.level ?? 'L1') as 'L0' | 'L1' | 'L2' | 'L3',
        );
        return {
          commandId: cmd.commandId,
          status: 'completed',
          data: summary,
          completedAt: new Date().toISOString(),
        };
      }
      const summaries = await this.ctx.summaryService.summarizeProject(
        this.ctx.rootPath,
        (p.level ?? 'L1') as 'L0' | 'L1' | 'L2' | 'L3',
      );
      return {
        commandId: cmd.commandId,
        status: 'completed',
        data: { files: summaries.length, summaries },
        completedAt: new Date().toISOString(),
      };
    });
  }

  // ── Push all state ─────────────────────────────────

  private async pushAll(): Promise<void> {
    if (!this.projectId) return;

    // Push health, tokens, swarm, graph in parallel
    await Promise.allSettled([
      this.pushHealth(),
      this.pushTokens(),
      this.pushSwarm(),
      this.pushGraph(),
    ]);
  }

  private async pushHealth(): Promise<void> {
    try {
      const result = await this.ctx.archAnalyzer.analyzeArchitecture('.');
      await this.pushState('health', result);
    } catch (err) {
      this.log('health push failed:', err);
    }
  }

  private async pushTokens(): Promise<void> {
    try {
      const globResults = await Promise.all([
        this.ctx.fs.glob('**/*.ts'),
        this.ctx.fs.glob('**/*.go'),
        this.ctx.fs.glob('**/*.rs'),
      ]);
      const allFiles = globResults.flat();
      const sourceFiles = allFiles.filter(
        (f) => !f.includes('node_modules') && !f.includes('dist')
          && !f.includes('.test.ts') && !f.includes('.spec.ts')
          && !f.includes('_test.go') && !f.includes('.test.rs')
          && !f.includes('examples'),
      );

      const files = await Promise.all(
        sourceFiles.map(async (filePath) => {
          const [l0, l1, l2, l3] = await Promise.all([
            this.ctx.ast.extractSummary(filePath, 'L0'),
            this.ctx.ast.extractSummary(filePath, 'L1'),
            this.ctx.ast.extractSummary(filePath, 'L2'),
            this.ctx.ast.extractSummary(filePath, 'L3'),
          ]);
          return {
            path: filePath,
            l0Tokens: l0.tokenEstimate,
            l1Tokens: l1.tokenEstimate,
            l2Tokens: l2.tokenEstimate,
            l3Tokens: l3.tokenEstimate,
            // Compression ratio: fraction of tokens saved (0-1).
            // L1 is the summary, L3 is the full source.
            ratio: l3.tokenEstimate > 0 ? +(1 - l1.tokenEstimate / l3.tokenEstimate).toFixed(3) : 0,
            lineCount: l1.lineCount,
          };
        }),
      );

      // Push overview (file list with ratios)
      await this.pushState('tokens', { files });

      // Push per-file token levels so the dashboard can render L0-L3 bars
      await Promise.all(
        files.map((f) =>
          this.post('/api/push', {
            projectId: this.projectId,
            type: 'tokenFile',
            filePath: f.path,
            data: {
              l0: { tokens: f.l0Tokens },
              l1: { tokens: f.l1Tokens },
              l2: { tokens: f.l2Tokens },
              l3: { tokens: f.l3Tokens },
            },
          }),
        ),
      );
    } catch (err) {
      this.log('tokens push failed:', err);
    }
  }

  private async pushSwarm(): Promise<void> {
    try {
      const [status, tasks, agents] = await Promise.all([
        this.ctx.swarm.status(),
        this.ctx.swarm.listTasks(),
        this.ctx.swarm.listAgents(),
      ]);
      await this.pushState('swarm', { status, tasks, agents });
    } catch (err) {
      // Swarm may not be running — push empty state
      await this.pushState('swarm', {
        status: { status: 'idle', agentCount: 0, activeTaskCount: 0, completedTaskCount: 0 },
        tasks: [],
        agents: [],
      }).catch(() => {});
    }
  }

  private async pushGraph(): Promise<void> {
    try {
      const edges: ImportEdge[] = await this.ctx.archAnalyzer.buildDependencyGraph(this.ctx.rootPath);
      const nodeSet = new Set<string>();
      for (const e of edges) { nodeSet.add(e.from); nodeSet.add(e.to); }
      const nodes = Array.from(nodeSet).map((id) => ({ id, layer: classifyLayer(id) }));
      const graphEdges = edges.map((e) => ({ from: e.from, to: e.to, names: e.names }));
      await this.pushState('graph', { nodes, edges: graphEdges });
    } catch (err) {
      this.log('graph push failed:', err);
    }
  }

  // ── File Watcher ───────────────────────────────────

  private startFileWatcher(): void {
    const srcDir = resolve(this.ctx.rootPath, 'src');
    try {
      const watcher = watch(srcDir, { recursive: true }, (_eventType, filename) => {
        if (!filename) return;
        if (!filename.endsWith('.ts') && !filename.endsWith('.js') && !filename.endsWith('.go') && !filename.endsWith('.rs')) return;

        const relPath = `src/${filename}`;
        const existing = this.fileChangeDebounce.get(relPath);
        if (existing) clearTimeout(existing);

        this.fileChangeDebounce.set(relPath, setTimeout(() => {
          this.fileChangeDebounce.delete(relPath);
          void this.pushEvent('file-change', {
            path: relPath,
            layer: classifyLayer(relPath),
            timestamp: Date.now(),
          });
        }, 300));
      });
      this.watchers.push(watcher);
    } catch {
      // src/ may not exist yet — that's fine
    }
  }

  // ── HTTP helpers (push to hub) ─────────────────────

  private pushState(type: string, data: unknown): Promise<void> {
    return this.post('/api/push', {
      projectId: this.projectId,
      type,
      data,
    }).then(() => {});
  }

  private pushEvent(event: string, data: unknown): Promise<void> {
    return this.post('/api/event', {
      projectId: this.projectId,
      event,
      data,
    }).then(() => {});
  }

  /** POST JSON to the hub. Returns parsed response or null on failure. */
  private post(path: string, body: unknown): Promise<Record<string, unknown> | null> {
    return new Promise((resolve) => {
      const payload = JSON.stringify(body);
      const headers: Record<string, string | number> = {
        'Content-Type': 'application/json',
        'Content-Length': Buffer.byteLength(payload),
      };
      if (this.authToken) {
        headers['Authorization'] = `Bearer ${this.authToken}`;
      }
      const req = request(
        {
          hostname: '127.0.0.1',
          port: this.hubPort,
          path,
          method: 'POST',
          headers,
          timeout: 5000,
        },
        (res) => {
          const chunks: Buffer[] = [];
          res.on('data', (c: Buffer) => chunks.push(c));
          res.on('end', () => {
            try {
              const raw = JSON.parse(Buffer.concat(chunks).toString('utf-8'));
              resolve(raw as Record<string, unknown>);
            } catch {
              resolve(null);
            }
          });
        },
      );
      req.on('error', () => resolve(null));
      req.on('timeout', () => { req.destroy(); resolve(null); });
      req.end(payload);
    });
  }

  /** Broadcast to local SSE clients (kept for backward compat). */
  broadcast(event: string, data: unknown): void {
    // Forward to hub as an event
    if (this.projectId) {
      void this.pushEvent(event, data);
    }
  }

  private log(msg: string, err?: unknown): void {
    const errMsg = err instanceof Error ? err.message : String(err ?? '');
    process.stderr.write(`[dashboard-client] ${msg} ${errMsg}\n`);
  }
}

// ── Convenience factory ──────────────────────────────────

export async function startDashboard(
  ctx: AppContext,
  hubPort?: number,
): Promise<{ url: string; close: () => void; commandReceiver: IHubCommandReceiverPort }> {
  const adapter = new DashboardAdapter(ctx, hubPort);
  const handle = await adapter.start();
  return { ...handle, commandReceiver: adapter };
}
