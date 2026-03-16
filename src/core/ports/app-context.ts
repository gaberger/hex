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
import type { ISwarmPort, ISwarmOrchestrationPort } from './swarm.js';
import type { IRegistryPort } from './registry.js';
import type { IBroadcastPort } from './broadcast.js';
import type { ISecretsPort } from './secrets.js';
import type { ICheckpointPort } from './checkpoint.js';

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

  // Swarm orchestration (composes swarm + worktree for parallel execution)
  swarmOrchestrator: ISwarmOrchestrationPort;

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
  broadcaster: IBroadcastPort;

  /** Secrets manager — Infisical when configured, env-var fallback otherwise */
  secrets: ISecretsPort;

  /** Checkpoint persistence for crash-resilient state recovery */
  checkpoint: ICheckpointPort;

  /** Local output directory for analysis reports, caches, and logs */
  outputDir: string; // defaults to '.hex/' — gitignored, project-scoped
}

/** Factory that creates an AppContext for a given project root path. */
export type AppContextFactory = (rootPath: string) => Promise<AppContext>;
