/**
 * Application Context Port
 *
 * Defines the shape of the fully-wired application context.
 * The composition root IMPLEMENTS this; primary adapters CONSUME it.
 * This prevents adapters from importing the composition root directly.
 */

import type { IArchAnalysisPort, IASTPort, ICodeGenerationPort, IFileSystemPort, IGitPort, ILLMPort, ISummaryPort, IWorktreePort, IWorkplanPort, IBuildPort } from './index.js';
import type { INotificationEmitPort, INotificationQueryPort } from './notification.js';
import type { IEventBusPort } from './event-bus.js';
import type { ISwarmPort } from './swarm.js';
import type { IRegistryPort } from './registry.js';

export interface AppContext {
  rootPath: string;
  astIsStub: boolean;

  /** When true, skip interactive prompts and use sensible defaults (--yes / -y) */
  autoConfirm: boolean;

  // Use cases
  archAnalyzer: IArchAnalysisPort;
  summaryService: ISummaryPort;
  notificationOrchestrator: INotificationQueryPort | null;

  // LLM-powered (null when no API key)
  llm: ILLMPort | null;
  codeGenerator: ICodeGenerationPort | null;
  workplanExecutor: IWorkplanPort | null;

  // Secondary adapters (behind port interfaces)
  fs: IFileSystemPort;
  git: IGitPort;
  worktree: IWorktreePort;
  build: IBuildPort;
  ast: IASTPort;
  eventBus: IEventBusPort | null;
  notifier: INotificationEmitPort;
  swarm: ISwarmPort;
  registry: IRegistryPort;

  /** Local output directory for analysis reports, caches, and logs */
  outputDir: string; // defaults to '.hex-intf/' — gitignored, project-scoped
}
