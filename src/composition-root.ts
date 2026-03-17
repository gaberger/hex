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
import { ADRAdapter } from './adapters/secondary/adr-adapter.js';
import { ADROrchestrator } from './core/usecases/adr-orchestrator.js';

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
import { AnthropicAgentAdapter } from './adapters/secondary/anthropic-agent-adapter.js';
import { ClaudeCodeExecutorAdapter } from './adapters/secondary/claude-code-executor-adapter.js';
import type { IAgentExecutorPort } from './core/ports/agent-executor.js';
import { InMemoryEventBus } from './adapters/secondary/in-memory-event-bus.js';

import { EnvSecretsAdapter } from './adapters/secondary/env-secrets-adapter.js';
import { InfisicalAdapter } from './adapters/secondary/infisical-adapter.js';
import { LocalVaultAdapter } from './adapters/secondary/local-vault-adapter.js';
import { HubLauncher } from './adapters/secondary/hub-launcher.js';
import { VersionAdapter } from './adapters/secondary/version-adapter.js';
import { CachingSecretsAdapter } from './adapters/secondary/caching-secrets-adapter.js';
import { FileCheckpointAdapter } from './adapters/secondary/file-checkpoint-adapter.js';
import type { ISecretsPort } from './core/ports/secrets.js';

// Re-export AppContext from the port (canonical definition)
export type { AppContext } from './core/ports/app-context.js';

// ── Secrets Factory ─────────────────────────────────────
// Centralised here (the only file allowed to import adapters).

interface SecretsConfig {
  version: 1;
  backend: 'infisical' | 'local-vault' | 'env';
  infisical?: {
    siteUrl: string;
    projectId: string;
    defaultEnvironment?: string;
    auth: {
      method: 'universal-auth';
      clientId: string;
      clientSecret: string;
    };
  };
  localVault?: {
    path?: string;
  };
  cache?: {
    ttlSeconds?: number;
  };
}

interface BuildSecretsOptions {
  /** Override vault password instead of reading from process.env (useful for testing). */
  vaultPassword?: string;
}

/**
 * Build the correct ISecretsPort from the project's `.hex/secrets.json`.
 *
 * Falls back to EnvSecretsAdapter when no config exists, config is
 * invalid, or the requested backend cannot be initialised.
 */
export async function buildSecretsAdapter(
  projectRoot: string,
  options?: BuildSecretsOptions,
): Promise<EnvSecretsAdapter | CachingSecretsAdapter | LocalVaultAdapter> {
  const { existsSync, readFileSync } = await import('node:fs');
  const { resolve } = await import('node:path');
  const configPath = resolve(projectRoot, '.hex/secrets.json');

  if (!existsSync(configPath)) {
    return new EnvSecretsAdapter();
  }

  let config: SecretsConfig;
  try {
    const raw = readFileSync(configPath, 'utf-8');
    config = JSON.parse(raw) as SecretsConfig;
  } catch {
    console.warn(`[hex] Warning: invalid JSON in ${configPath} — falling back to env secrets`);
    return new EnvSecretsAdapter();
  }

  switch (config.backend) {
    case 'infisical': {
      const inf = config.infisical;
      if (!inf) {
        console.warn('[hex] Warning: backend is "infisical" but no infisical config — falling back to env');
        return new EnvSecretsAdapter();
      }
      const adapter = new InfisicalAdapter({
        siteUrl: inf.siteUrl,
        clientId: inf.auth.clientId,
        clientSecret: inf.auth.clientSecret,
        projectId: inf.projectId,
        defaultEnvironment: inf.defaultEnvironment,
      });
      const ttl = (config.cache?.ttlSeconds ?? 300) * 1000;
      return new CachingSecretsAdapter(adapter, ttl);
    }

    case 'local-vault': {
      const { resolve: res } = await import('node:path');
      const vaultRelPath = config.localVault?.path ?? '.hex/vault.enc';
      const vaultPath = res(projectRoot, vaultRelPath);
      const { existsSync: exists } = await import('node:fs');

      if (!exists(vaultPath)) {
        console.warn(`[hex] Warning: vault file not found at ${vaultPath} — falling back to env secrets`);
        return new EnvSecretsAdapter();
      }

      const password = options?.vaultPassword ?? process.env['HEX_VAULT_PASSWORD'];
      if (!password) {
        console.warn('[hex] Warning: HEX_VAULT_PASSWORD not set — falling back to env secrets');
        return new EnvSecretsAdapter();
      }

      return new LocalVaultAdapter(vaultPath, password);
    }

    case 'env':
    default:
      return new EnvSecretsAdapter();
  }
}

// ── Factory ─────────────────────────────────────────────

export interface CreateAppContextOptions {
  /** Optional callback to prompt for the vault password interactively (CLI only). */
  getVaultPassword?: () => Promise<string>;
}

export async function createAppContext(
  projectPath: string,
  options?: CreateAppContextOptions,
): Promise<AppContext> {
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
  const checkpoint = new FileCheckpointAdapter(`${outputDir}/checkpoints`, fs);

  // Cross-language communication adapters (dynamic imports to avoid circular init)
  const { SerializationAdapter } = await import('./adapters/secondary/serialization-adapter.js');
  const { JsonSchemaAdapter } = await import('./adapters/secondary/json-schema-adapter.js');
  const { ScaffoldService } = await import('./core/usecases/scaffold-service.js');
  const { WASMBridgeAdapter } = await import('./adapters/secondary/wasm-bridge-adapter.js');
  const { FFIAdapter } = await import('./adapters/secondary/ffi-adapter.js');
  const { HTTPServiceMeshAdapter } = await import('./adapters/secondary/service-mesh-adapter.js');

  const { ValidationAdapter } = await import('./adapters/secondary/validation-adapter.js');

  const serialization = new SerializationAdapter();
  const schema = new JsonSchemaAdapter();
  const scaffold = new ScaffoldService(fs);
  const wasmBridge = new WASMBridgeAdapter();
  const ffi = new FFIAdapter();
  const serviceMesh = new HTTPServiceMeshAdapter();
  const validator = new ValidationAdapter();

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
  // Use cases
  const archAnalyzer = new ArchAnalyzer(ast, fs, git);
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

    // Checkpoint recovery — non-blocking, log-only
    void checkpoint.recover(projectName).then(
      (entry) => {
        if (entry) {
          const featureCount = entry.features.length;
          const orphanCount = entry.orphanTasks.length;
          process.stderr.write(`[hex] Checkpoint found from ${entry.createdAt}: ${featureCount} feature(s), ${orphanCount} orphan task(s)\n`);
        }
      },
      () => { /* recovery failed — non-critical */ },
    );

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
        const ctx = { rootPath: projectPath, astIsStub, archAnalyzer, ast, fs, git, worktree, build, swarm, registry, notifier, eventBus, summaryService, notificationOrchestrator, llm: null, codeGenerator: null, workplanExecutor: null, swarmOrchestrator, autoConfirm: false, outputDir } as any;
        const client = new DashboardAdapter(ctx, HUB_PORT);
        const { url } = await client.start();
        status.dashboard = url.replace('http://', '');
        // Deterministic project ID (must match Rust hex-hub's make_project_id)
        const basename = projectPath.split('/').pop() ?? 'unknown';
        const hash = Array.from(projectPath).reduce((h, c) => ((h << 5) - h + c.charCodeAt(0)) | 0, 0);
        const projId = `${basename}-${(hash >>> 0).toString(36)}`;
        status.projectId = projId;
        process.stderr.write(`[hex] Project registered with dashboard hub\n`);

        // Start coordination instance (multi-session awareness)
        try {
          const { CoordinationAdapter } = await import('./adapters/secondary/coordination-adapter.js');
          const coord = new CoordinationAdapter(projId, projectPath, HUB_PORT);
          const instanceId = await coord.registerInstance();
          status.coordinationInstanceId = instanceId;
          // Wire into AppContext so CLI/MCP/dashboard can access coordination
          (appContext as any).coordination = coord;
          process.stderr.write(`[hex] Coordination registered: ${instanceId.slice(0, 8)}…\n`);
        } catch (coordErr) {
          process.stderr.write(`[hex] Coordination skipped: ${coordErr instanceof Error ? coordErr.message : String(coordErr)}\n`);
        }
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
  const secrets: ISecretsPort = await buildSecretsAdapter(projectPath);

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
    workplanExecutor = new WorkplanExecutor(llm, ast, fs, swarm, codeGenerator);
  }

  // Agent executors: Anthropic API (direct) + Claude Code CLI (baseline)
  let anthropicExecutor: IAgentExecutorPort | null = null;
  let claudeCodeExecutor: IAgentExecutorPort | null = null;

  if (anthropicKey) {
    anthropicExecutor = new AnthropicAgentAdapter(
      { apiKey: anthropicKey, model: 'claude-sonnet-4-20250514' },
      fs,
    );
  }
  // Claude Code executor always available if binary is installed
  claudeCodeExecutor = new ClaudeCodeExecutorAdapter({}, fs);

  // Dual-swarm comparator — available when both executors exist
  let comparator: import('./core/ports/agent-executor.js').IComparisonPort | null = null;
  if (anthropicExecutor && claudeCodeExecutor) {
    const { DualSwarmComparator } = await import('./core/usecases/dual-swarm-comparator.js');
    comparator = new DualSwarmComparator({
      claudeCodeExecutor,
      anthropicApiExecutor: anthropicExecutor,
      build,
      archAnalyzer,
      worktree,
    });
  }

  // ADR lifecycle tracking — always available (gracefully handles missing docs/adrs/)
  const adrAdapter = new ADRAdapter(fs, swarm);
  const adrQuery = new ADROrchestrator(adrAdapter, worktree);

  // Background ADR reindex (non-blocking, best-effort)
  if (!isTest) {
    void adrAdapter.indexIntoAgentDB().catch(() => {});
  }

  // Version + Hub launcher — available to CLI/MCP via ports
  const version = new VersionAdapter();
  const hubLauncher = new HubLauncher();

  // Vault manager — always available for `hex secrets init` (createVault).
  // When a vault is open, addSecret/removeSecret also work.
  const vaultManager = secrets instanceof LocalVaultAdapter
    ? secrets
    : {
        createVault: (path: string, pw: string) => LocalVaultAdapter.createVault(path, pw),
        addSecret() { throw new Error('No vault open — run `hex secrets init` first'); },
        removeSecret() { throw new Error('No vault open — run `hex secrets init` first'); },
      };

  const appContext: AppContext = {
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
    secrets,
    checkpoint,
    scaffold,
    validator,
    serialization,
    wasmBridge,
    ffi,
    serviceMesh,
    schema,
    version,
    hubLauncher,
    vaultManager,
    adrQuery,
    coordination: null, // wired dynamically via CoordinationAdapter when hub is available
    hubCommandSender: null, // wired dynamically when hub is available
    createDashboard: async (rootPath: string) => {
      const { DashboardAdapter } = await import('./adapters/primary/dashboard-adapter.js');
      const dashCtx = {
        rootPath,
        astIsStub,
        autoConfirm: false,
        archAnalyzer,
        ast,
        fs,
        git,
        worktree,
        build,
        swarm,
        registry,
        notifier,
        eventBus,
        summaryService,
        notificationOrchestrator,
        llm: null,
        codeGenerator: null,
        workplanExecutor: null,
        swarmOrchestrator,
        outputDir,
      } as any;
      return new DashboardAdapter(dashCtx);
    },
    anthropicExecutor,
    claudeCodeExecutor,
    comparator,
  } as AppContext;

  return appContext;
}
