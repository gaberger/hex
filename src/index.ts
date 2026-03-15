/**
 * hex-intf — Public Library API
 *
 * Re-exports port interfaces, domain entities, and the composition root
 * factory for npm consumers. Adapters are NOT exported (they are internal).
 */

// ── Port Interfaces ─────────────────────────────────────
export type {
  IASTPort,
  IArchAnalysisPort,
  IBuildPort,
  IFileSystemPort,
  IGitPort,
  ILLMPort,
  IWorktreePort,
  ICodeGenerationPort,
  ISummaryPort,
  IWorkplanPort,
  Language,
  ASTSummary,
  ArchAnalysisResult,
  DependencyViolation,
  ImportEdge,
  DeadExport,
  CodeUnit,
  BuildResult,
  LintResult,
  TestResult,
} from './core/ports/index.js';

export type { IEventBusPort } from './core/ports/event-bus.js';

export type {
  ISwarmPort,
  SwarmConfig,
  SwarmStatus,
  SwarmTask,
} from './core/ports/swarm.js';

export type {
  INotificationEmitPort,
  INotificationQueryPort,
  Notification,
  ProgressReport,
  StatusLine,
} from './core/ports/notification.js';

// ── Domain Entities ─────────────────────────────────────
export { QualityScore, FeedbackLoop, TaskGraph } from './core/domain/entities.js';

// ── Composition Root ────────────────────────────────────
export { createAppContext } from './composition-root.js';
export type { AppContext } from './composition-root.js';
