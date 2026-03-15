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
  IFileSystemPort,
  IGitPort,
  IWorktreePort,
} from './core/ports/index.js';
import type { IEventBusPort } from './core/ports/event-bus.js';
import type { INotificationEmitPort } from './core/ports/notification.js';
import { ArchAnalyzer } from './core/usecases/arch-analyzer.js';
import { NotificationOrchestrator } from './core/usecases/notification-orchestrator.js';

// ── Secondary Adapters (the ONLY adapter imports in the entire project) ──
import { FileSystemAdapter } from './adapters/secondary/filesystem-adapter.js';
import { TreeSitterAdapter } from './adapters/secondary/treesitter-adapter.js';
import { TerminalNotifier } from './adapters/secondary/terminal-notifier.js';
import { GitAdapter } from './adapters/secondary/git-adapter.js';
import { WorktreeAdapter } from './adapters/secondary/worktree-adapter.js';
import { BuildAdapter } from './adapters/secondary/build-adapter.js';
import { RufloAdapter } from './adapters/secondary/ruflo-adapter.js';

// ── AppContext ───────────────────────────────────────────

export interface AppContext {
  rootPath: string;

  // Use cases (primary ports)
  archAnalyzer: IArchAnalysisPort;
  notificationOrchestrator: NotificationOrchestrator;

  // Secondary adapters
  fs: IFileSystemPort;
  git: IGitPort;
  worktree: IWorktreePort;
  build: IBuildPort;
  ast: IASTPort;
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

  // Tree-sitter: best-effort — falls back to a stub if WASM grammars missing
  let ast: IASTPort;
  try {
    const grammarDir = `${projectPath}/node_modules/web-tree-sitter`;
    ast = await TreeSitterAdapter.create(grammarDir, fs);
  } catch {
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

  return {
    rootPath: projectPath,
    archAnalyzer,
    notificationOrchestrator,
    fs,
    git,
    worktree,
    build,
    ast,
    eventBus: NULL_EVENT_BUS,
    notifier,
    swarm,
  };
}
