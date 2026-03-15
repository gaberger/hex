/**
 * Composition Root
 *
 * The ONLY file in the project that imports from both ports AND adapters.
 * Wires secondary adapters into use cases via constructor injection and
 * returns a fully assembled AppContext.
 */

import type {
  IASTPort,
  IArchAnalysisPort,
  IBuildPort,
  ICodeGenerationPort,
  IFileSystemPort,
  IGitPort,
  ILLMPort,
  ISummaryPort,
  IWorkplanPort,
  IWorktreePort,
} from './core/ports/index.js';
import type { IEventBusPort } from './core/ports/event-bus.js';
import type { INotificationEmitPort } from './core/ports/notification.js';
import { ArchAnalyzer } from './core/usecases/arch-analyzer.js';
import { NotificationOrchestrator } from './core/usecases/notification-orchestrator.js';
import { CodeGenerator } from './core/usecases/code-generator.js';
import { WorkplanExecutor } from './core/usecases/workplan-executor.js';
import { SummaryService } from './core/usecases/summary-service.js';

// ── Secondary Adapters (the ONLY adapter imports in the entire project) ──
import { FileSystemAdapter } from './adapters/secondary/filesystem-adapter.js';
import { TreeSitterAdapter } from './adapters/secondary/treesitter-adapter.js';
import { TerminalNotifier } from './adapters/secondary/terminal-notifier.js';
import { GitAdapter } from './adapters/secondary/git-adapter.js';
import { WorktreeAdapter } from './adapters/secondary/worktree-adapter.js';
import { BuildAdapter } from './adapters/secondary/build-adapter.js';
import { RufloAdapter } from './adapters/secondary/ruflo-adapter.js';
import { LLMAdapter } from './adapters/secondary/llm-adapter.js';
import type { LLMAdapterConfig } from './adapters/secondary/llm-adapter.js';

// ── AppContext ───────────────────────────────────────────

export interface AppContext {
  rootPath: string;

  // Use cases (primary ports)
  archAnalyzer: IArchAnalysisPort;
  notificationOrchestrator: NotificationOrchestrator;

  // LLM-powered use cases (null when no API key is configured)
  llm: ILLMPort | null;
  codeGenerator: ICodeGenerationPort | null;
  workplanExecutor: IWorkplanPort | null;
  summaryService: ISummaryPort;

  // Secondary adapters
  fs: IFileSystemPort;
  git: IGitPort;
  worktree: IWorktreePort;
  build: IBuildPort;
  ast: IASTPort;
  astIsStub: boolean;
  eventBus: IEventBusPort | null;
  notifier: INotificationEmitPort;
  swarm: import('./core/ports/swarm.js').ISwarmPort;
}

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
  // Secondary adapters — all real implementations
  const fs = new FileSystemAdapter(projectPath);
  const git = new GitAdapter(projectPath);
  const worktree = new WorktreeAdapter(projectPath, `${projectPath}/../hex-worktrees`);
  const build = new BuildAdapter(projectPath);
  const notifier = new TerminalNotifier();
  const swarm = new RufloAdapter(projectPath);

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
  } catch {
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
    codeGenerator = new CodeGenerator(llm, ast, build, fs);
    workplanExecutor = new WorkplanExecutor(llm, ast, fs, swarm);
  }

  return {
    rootPath: projectPath,
    archAnalyzer,
    notificationOrchestrator,
    llm,
    codeGenerator,
    workplanExecutor,
    summaryService,
    fs,
    git,
    worktree,
    build,
    ast,
    astIsStub,
    eventBus: NULL_EVENT_BUS,
    notifier,
    swarm,
  };
}
