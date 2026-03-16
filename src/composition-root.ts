/**
 * Composition Root
 *
 * The ONLY file in the project that imports from both ports AND adapters.
 * Wires secondary adapters into use cases via constructor injection and
 * returns a fully assembled AppContext.
 */

import type { AppContext } from './core/ports/app-context.js';
import type { IASTPort, ILLMPort, ICodeGenerationPort, IWorkplanPort } from './core/ports/index.js';
import type { IEventBusPort } from './core/ports/event-bus.js';
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

// Re-export AppContext from the port (canonical definition)
export type { AppContext } from './core/ports/app-context.js';

// ── Null Event Bus (only remaining stub — real impl comes with hive-mind) ──

const NULL_EVENT_BUS: IEventBusPort = {
  async publish() {},
  subscribe() { return { id: 'noop', unsubscribe() {} }; },
  subscribeFiltered() { return { id: 'noop', unsubscribe() {} }; },
  subscribeAll() { return { id: 'noop', unsubscribe() {} }; },
  async getHistory() { return []; },
  reset() {},
};

// ── Factory ─────────────────────────────────────────────

export async function createAppContext(projectPath: string): Promise<AppContext> {
  // Project-scoped output directory for analysis, caches, logs
  const outputDir = `${projectPath}/.hex-intf`;
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
    process.stderr.write(`[hex-intf] WARNING: Tree-sitter init failed: ${msg}. Analysis will return empty results.\n`);
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

  // Use cases
  const archAnalyzer = new ArchAnalyzer(ast, fs);
  const notificationOrchestrator = new NotificationOrchestrator(notifier);
  const summaryService = new SummaryService(ast, fs);
  const swarmOrchestrator = new SwarmOrchestrator(swarm, worktree);

  // ── Initialize swarm + AgentDB on startup ──────────────
  // The swarm should be ready when the project loads, not lazily.
  // AgentDB session tracks this project's lifecycle for pattern learning.
  const projectName = projectPath.split('/').pop() ?? 'unknown';
  try {
    await swarm.init({
      topology: 'hierarchical',
      maxAgents: 4,
      strategy: 'specialized',
      consensus: 'raft',
      memoryNamespace: `hex-intf:${projectName}`,
    });
    process.stderr.write(`[hex-intf] Swarm initialized for ${projectName}\n`);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    process.stderr.write(`[hex-intf] WARNING: Swarm init failed: ${msg}. Orchestration will be unavailable.\n`);
  }

  try {
    await swarm.sessionStart(`hex-intf:${projectName}`, {
      projectPath,
      startedAt: new Date().toISOString(),
    });
    process.stderr.write(`[hex-intf] AgentDB session started for ${projectName}\n`);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    process.stderr.write(`[hex-intf] WARNING: AgentDB session start failed: ${msg}. Pattern learning unavailable.\n`);
  }

  // LLM: graceful degradation — null when no API key is configured
  let llm: ILLMPort | null = null;
  let codeGenerator: ICodeGenerationPort | null = null;
  let workplanExecutor: IWorkplanPort | null = null;

  const anthropicKey = process.env['ANTHROPIC_API_KEY'];
  const openaiKey = process.env['OPENAI_API_KEY'];

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
    eventBus: NULL_EVENT_BUS,
    notifier,
    swarm,
    registry,
  };
}
