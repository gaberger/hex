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

import type { ISecretsPort } from './secrets.js';
import type { ICheckpointPort } from './checkpoint.js';
import type { IScaffoldPort } from './scaffold.js';
import type { IValidationPort } from './validation.js';
import type { ISerializationPort, IWASMBridgePort, IFFIPort, IServiceMeshPort, ISchemaPort } from './cross-lang.js';
import type { IHubCommandSenderPort } from './hub-command.js';
import type { IAgentExecutorPort, IComparisonPort } from './agent-executor.js';
import type { IVersionPort } from './index.js';
import type { IHubLauncherPort } from './hub-launcher.js';
import type { IVaultManagementPort } from './vault.js';
import type { ICoordinationPort } from './coordination.js';
import type { IADRQueryPort } from './adr.js';

/** Minimal interface for a dashboard client (avoids adapter-to-adapter imports). */
export interface IDashboardClient {
  start(): Promise<{ url: string; close: () => void }>;
  stop(): void;
}

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
  /** Secrets manager — Infisical when configured, env-var fallback otherwise */
  secrets: ISecretsPort;

  /** Checkpoint persistence for crash-resilient state recovery */
  checkpoint: ICheckpointPort;

  /** Scaffold & runtime analysis for generated projects */
  scaffold: IScaffoldPort;

  /** Post-build semantic validation (behavioral specs, property tests) */
  validator: IValidationPort | null;

  /** Cross-language serialization (JSON, future: Protobuf, MessagePack) */
  serialization: ISerializationPort;

  /** WebAssembly module loading and calling */
  wasmBridge: IWASMBridgePort | null;

  /** Foreign Function Interface for native binaries */
  ffi: IFFIPort | null;

  /** HTTP service mesh for cross-language service discovery */
  serviceMesh: IServiceMeshPort | null;

  /** JSON Schema validation for cross-language type contracts */
  schema: ISchemaPort;

  /** CLI/hub version information */
  version: IVersionPort;

  /** hex-hub daemon lifecycle (null when binary not installed) */
  hubLauncher: IHubLauncherPort | null;

  /** Vault CRUD for `hex secrets` commands — always available (factory-only when no vault open) */
  vaultManager: IVaultManagementPort;

  /** Send commands to hex-hub (null when hub is not available) */
  hubCommandSender: IHubCommandSenderPort | null;

  /** Multi-instance coordination via hex-hub (null when hub unavailable) */
  coordination: ICoordinationPort | null;

  /** Agent executors for direct API comparison (null when no API key) */
  anthropicExecutor: IAgentExecutorPort | null;
  claudeCodeExecutor: IAgentExecutorPort | null;

  /** Dual-swarm comparator (null when both executors are not available) */
  comparator: IComparisonPort | null;

  /** ADR lifecycle queries (null when ADR directory doesn't exist) */
  adrQuery: IADRQueryPort | null;

  /** Local output directory for analysis reports, caches, and logs */
  outputDir: string; // defaults to '.hex/' — gitignored, project-scoped

  /** Factory to create a dashboard client (wired in composition root to avoid cross-adapter imports). */
  createDashboard?: (rootPath: string) => Promise<IDashboardClient>;
}

/** Factory that creates an AppContext for a given project root path. */
type AppContextFactory = (rootPath: string) => Promise<AppContext>;
