/**
 * Composition Root
 *
 * The ONLY file in the project that imports from both ports AND adapters.
 * Wires secondary adapters into use cases via constructor injection and
 * returns a fully assembled AppContext.
 */

import type { AppContext } from './core/ports/app-context.js';
import type { IASTPort, ILLMPort, ICodeGenerationPort, IWorkplanPort } from './core/ports/index.js';
import { ArchAnalyzer } from './core/usecases/arch-analyzer.js';
import { NotificationOrchestrator } from './core/usecases/notification-orchestrator.js';
import { CodeGenerator } from './core/usecases/code-generator.js';
import { WorkplanExecutor } from './core/usecases/workplan-executor.js';
import { SummaryService } from './core/usecases/summary-service.js';
import { SwarmOrchestrator } from './core/usecases/swarm-orchestrator.js';

// ── Secondary Adapters (the ONLY adapter imports in the entire project) ──
import { FileSystemAdapter } from './adapters/secondary/filesystem-adapter.js';
import { TreeSitterAdapter } from './adapters/secondary/treesitter-adapter.js';
import { TerminalNotifier } from './adapters/secondary/terminal-notifier.js';
import { GitAdapter } from './adapters/secondary/git-adapter.js';
import { WorktreeAdapter } from './adapters/secondary/worktree-adapter.js';
import { BuildAdapter } from './adapters/secondary/build-adapter.js';
import { RufloAdapter } from './adapters/secondary/ruflo-adapter.js';
import { RegistryAdapter } from './adapters/secondary/registry-adapter.js';
import { LLMAdapter } from './adapters/secondary/llm-adapter.js';
import type { LLMAdapterConfig } from './adapters/secondary/llm-adapter.js';
import { InMemoryEventBus } from './adapters/secondary/in-memory-event-bus.js';
import { SSEBroadcastAdapter } from './adapters/secondary/sse-broadcast-adapter.js';
import { EnvSecretsAdapter } from './adapters/secondary/env-secrets-adapter.js';
import { InfisicalAdapter } from './adapters/secondary/infisical-adapter.js';
import { LocalVaultAdapter } from './adapters/secondary/local-vault-adapter.js';
import { CachingSecretsAdapter } from './adapters/secondary/caching-secrets-adapter.js';
import type { ISecretsPort } from './core/ports/secrets.js';

// Re-export AppContext from the port (canonical definition)
export type { AppContext } from './core/ports/app-context.js';

// ── Factory ─────────────────────────────────────────────

export async function createAppContext(projectPath: string): Promise<AppContext> {
  // Project-scoped output directory for analysis, caches, logs
  const outputDir = `${projectPath}/.hex`;
  const { mkdir } = await import('node:fs/promises');
  // mkdir with recursive:true only fails on permission errors — safe to ignore
  await mkdir(outputDir, { recursive: true }).catch(() => {});

  // Secondary adapters — all real implementations
  const fs = new FileSystemAdapter(projectPath);
  const git = new GitAdapter(projectPath);
  const worktree = new WorktreeAdapter(projectPath, `${projectPath}/../hex-worktrees`);
  const build = new BuildAdapter(projectPath);
  const notifier = new TerminalNotifier();
  const swarm = new RufloAdapter(projectPath);
  const registry = new RegistryAdapter();

  // Tree-sitter: search multiple candidate directories for WASM grammars
  // Paths must be RELATIVE to project root — fs.exists() uses safePath()
  let ast: IASTPort;
  let astIsStub = false;
  try {
    const grammarDirs = [
      'config/grammars',
      'node_modules/tree-sitter-wasms/out',
      'node_modules/web-tree-sitter',
    ];
    const treeSitter = await TreeSitterAdapter.create(grammarDirs, fs, projectPath);
    if (treeSitter.isStub()) {
      astIsStub = true;
    }
    ast = treeSitter;
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    process.stderr.write(`[hex] WARNING: Tree-sitter init failed: ${msg}. Analysis will return empty results.\n`);
    astIsStub = true;
    ast = {
      async extractSummary(filePath, level) {
        return {
          filePath, language: 'typescript', level,
          exports: [], imports: [], dependencies: [],
          lineCount: 0, tokenEstimate: 0,
        };
      },
      diffStructural() { return { added: [], removed: [], modified: [] }; },
    };
  }

  // Event infrastructure — real pub/sub spine replacing the null stub
  const projectName = projectPath.split('/').pop() ?? 'unknown';
  const eventBus = new InMemoryEventBus();
  const broadcastAdapter = new SSEBroadcastAdapter();

  // Bridge: forward domain events to broadcast clients (SSE/WebSocket)
  eventBus.subscribeAll((event) => {
    broadcastAdapter.send({
      event: event.type,
      data: event,
      projectId: projectName,
    });
  });

  // Use cases
  const archAnalyzer = new ArchAnalyzer(ast, fs);
  const notificationOrchestrator = new NotificationOrchestrator(notifier);
  const summaryService = new SummaryService(ast, fs);
  const swarmOrchestrator = new SwarmOrchestrator(swarm, worktree);

  // ── Initialize swarm + AgentDB in background (non-blocking) ──
  // Skip during tests — npx child processes cause timeouts.
  // Skip when NODE_ENV=test or BUN_ENV=test (bun test sets this).
  const isTest = process.env['NODE_ENV'] === 'test' || process.env['BUN_ENV'] === 'test'
    || typeof (globalThis as any).Bun?.jest !== 'undefined';

  if (!isTest) {
    const { writeFile } = await import('node:fs/promises');
    const statusFile = `${outputDir}/status.json`;

    // Track what connects so the status line can read it
    const status: Record<string, unknown> = {
      project: projectName,
      projectPath,
      pid: process.pid,
      startedAt: new Date().toISOString(),
      swarm: false,
      agentdb: false,
      dashboard: null as string | null,
    };

    // Initialize swarm
    void swarm.init({
      topology: 'hierarchical',
      maxAgents: 4,
      strategy: 'specialized',
      consensus: 'raft',
      memoryNamespace: `hex:${projectName}`,
    }).then(
      () => { status.swarm = true; process.stderr.write(`[hex] Swarm initialized for ${projectName}\n`); },
      (err) => process.stderr.write(`[hex] Swarm init skipped: ${err instanceof Error ? err.message : String(err)}\n`),
    ).finally(() => writeFile(statusFile, JSON.stringify(status, null, 2)).catch(() => {}));

    // Start AgentDB session
    void swarm.sessionStart(`hex:${projectName}`, {
      projectPath,
      startedAt: new Date().toISOString(),
    }).then(
      () => { status.agentdb = true; process.stderr.write(`[hex] AgentDB session started for ${projectName}\n`); },
      (err) => process.stderr.write(`[hex] AgentDB session skipped: ${err instanceof Error ? err.message : String(err)}\n`),
    ).finally(() => writeFile(statusFile, JSON.stringify(status, null, 2)).catch(() => {}));

    // Start Rust hex-hub daemon (if binary available), then register as a client
    void (async () => {
      try {
        const { ensureHubRunning } = await import('./adapters/secondary/hub-launcher.js');
        try {
          const hubUrl = await ensureHubRunning();
          process.stderr.write(`[hex] Hub running at ${hubUrl}\n`);
        } catch (hubErr) {
          process.stderr.write(`[hex] Hub skipped: ${hubErr instanceof Error ? hubErr.message : String(hubErr)}\n`);
        }

        // Register this project as a client that pushes data to the hub
        const { DashboardAdapter } = await import('./adapters/primary/dashboard-adapter.js');
        const HUB_PORT = 5555;
        const ctx = { rootPath: projectPath, astIsStub, archAnalyzer, ast, fs, git, worktree, build, swarm, registry, notifier, eventBus, summaryService, notificationOrchestrator, llm: null, codeGenerator: null, workplanExecutor: null, swarmOrchestrator, autoConfirm: false, outputDir, broadcaster: broadcastAdapter } as any;
        const client = new DashboardAdapter(ctx, HUB_PORT);
        const { url } = await client.start();
        status.dashboard = url.replace('http://', '');
        process.stderr.write(`[hex] Project registered with dashboard hub\n`);
      } catch (err) {
        process.stderr.write(`[hex] Dashboard skipped: ${err instanceof Error ? err.message : String(err)}\n`);
      }
      await writeFile(statusFile, JSON.stringify(status, null, 2)).catch(() => {});
    })();

    // Periodic status updater — polls agent/task counts every 5s
    // Uses unref() so it doesn't keep the process alive
    const statusInterval = setInterval(async () => {
      try {
        const [agents, tasks] = await Promise.all([
          swarm.listAgents().catch(() => []),
          swarm.listTasks().catch(() => []),
        ]);
        status.activeAgents = agents.filter((a: any) => a.status === 'active').length;
        status.idleAgents = agents.filter((a: any) => a.status === 'idle' || a.status === 'spawning').length;
        status.tasks = tasks.length;
        status.completedTasks = tasks.filter((t: any) => t.status === 'completed').length;
        status.updatedAt = new Date().toISOString();
        await writeFile(statusFile, JSON.stringify(status, null, 2)).catch(() => {});
      } catch { /* swarm may not be available — skip silently */ }
    }, 5000);
    statusInterval.unref();
  }

  // Secrets: config-based adapter selection (.hex/secrets.json → Infisical/LocalVault/Env)
  let secrets: ISecretsPort = new EnvSecretsAdapter();
  try {
    const { existsSync, readFileSync } = await import('node:fs');
    const { resolve } = await import('node:path');
    const configPath = resolve(projectPath, '.hex/secrets.json');

    if (existsSync(configPath)) {
      const config = JSON.parse(readFileSync(configPath, 'utf-8'));

      if (config.backend === 'infisical' && config.infisical) {
        const inf = config.infisical;
        const adapter = new InfisicalAdapter({
          siteUrl: inf.siteUrl,
          clientId: inf.auth.clientId,
          clientSecret: inf.auth.clientSecret,
          projectId: inf.projectId,
          defaultEnvironment: inf.defaultEnvironment,
        });
        const ttl = (config.cache?.ttlSeconds ?? 300) * 1000;
        secrets = new CachingSecretsAdapter(adapter, ttl);
        process.stderr.write(`[hex] Secrets: Infisical (${inf.siteUrl})\n`);
      } else if (config.backend === 'local-vault') {
        const vaultPath = resolve(projectPath, config.localVault?.path ?? '.hex/vault.enc');
        const password = process.env['HEX_VAULT_PASSWORD'];
        if (existsSync(vaultPath) && password) {
          secrets = new LocalVaultAdapter(vaultPath, password);
          process.stderr.write('[hex] Secrets: Local vault\n');
        } else if (!existsSync(vaultPath)) {
          process.stderr.write(`[hex] Secrets: vault not found at ${vaultPath} — using env vars\n`);
        } else {
          process.stderr.write('[hex] Secrets: HEX_VAULT_PASSWORD not set — using env vars\n');
        }
      }
    }
  } catch (err) {
    process.stderr.write(`[hex] Secrets config error: ${err instanceof Error ? err.message : String(err)} — using env vars\n`);
  }

  // LLM: graceful degradation — null when no API key is configured
  // Try secrets port first (Infisical), fall back to direct env vars
  let llm: ILLMPort | null = null;
  let codeGenerator: ICodeGenerationPort | null = null;
  let workplanExecutor: IWorkplanPort | null = null;

  let anthropicKey = process.env['ANTHROPIC_API_KEY'];
  let openaiKey = process.env['OPENAI_API_KEY'];

  // If Infisical is active and env vars are missing, try resolving from secrets
  if (!anthropicKey && !openaiKey && !(secrets instanceof EnvSecretsAdapter)) {
    const [aResult, oResult] = await Promise.all([
      secrets.resolveSecret('ANTHROPIC_API_KEY'),
      secrets.resolveSecret('OPENAI_API_KEY'),
    ]);
    if (aResult.ok) anthropicKey = aResult.value;
    if (oResult.ok) openaiKey = oResult.value;
  }

  if (anthropicKey || openaiKey) {
    const provider: LLMAdapterConfig['provider'] = anthropicKey ? 'anthropic' : 'openai';
    const apiKey = (anthropicKey ?? openaiKey)!;
    const model = anthropicKey ? 'claude-sonnet-4-20250514' : 'gpt-4o';
    llm = new LLMAdapter({ provider, apiKey, model });
    codeGenerator = new CodeGenerator(llm, ast, build, fs, archAnalyzer);
    workplanExecutor = new WorkplanExecutor(llm, ast, fs, swarm);
  }

  return {
    rootPath: projectPath,
    autoConfirm: false,
    outputDir,
    archAnalyzer,
    notificationOrchestrator,
    llm,
    codeGenerator,
    workplanExecutor,
    summaryService,
    swarmOrchestrator,
    fs,
    git,
    worktree,
    build,
    ast,
    astIsStub,
    eventBus,
    notifier,
    swarm,
    registry,
    broadcaster: broadcastAdapter,
    secrets,
  };
}
