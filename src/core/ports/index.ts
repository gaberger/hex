/**
 * hex Port Interfaces
 *
 * All communication between domain core and adapters flows through these typed ports.
 * Input ports are implemented by use cases (driven by primary adapters).
 * Output ports are implemented by secondary adapters (driven by use cases).
 *
 * Value objects are defined in `../domain/value-objects.ts` and re-exported
 * here for public API stability. Ports depend on domain, not the reverse.
 */

// ─── Value Objects (re-exported from domain) ────────────

export type {
  Language,
  ASTSummary,
  ExportEntry,
  ImportEntry,
  TokenBudget,
  CodeUnit,
  Specification,
  Workplan,
  WorkplanStep,
  StepResult,
  LintError,
  BuildResult,
  LintResult,
  TestResult,
  TestFailure,
  StructuralDiff,
  WorktreePath,
  MergeResult,
  Message,
  LLMResponse,
  TestSuite,
  Project,
  DependencyDirection,
  ImportEdge,
  DeadExport,
  DependencyViolation,
  ArchAnalysisResult,
} from '../domain/value-objects.js';

export { Version } from '../domain/value-objects.js';

import type {
  ASTSummary,
  Language,
  CodeUnit,
  Specification,
  Workplan,
  StepResult,
  LintError,
  BuildResult,
  LintResult,
  TestResult,
  TokenBudget,
  Message,
  LLMResponse,
  StructuralDiff,
  WorktreePath,
  MergeResult,
  Project,
  TestSuite,
  DeadExport,
  DependencyViolation,
  ArchAnalysisResult,
  ImportEdge,
} from '../domain/value-objects.js';

import { Version } from '../domain/value-objects.js';

// ─── Input Ports (Primary / Driving) ─────────────────────

export interface ICodeGenerationPort {
  generateFromSpec(spec: Specification, lang: Language): Promise<CodeUnit>;
  refineFromFeedback(unit: CodeUnit, errors: LintError[]): Promise<CodeUnit>;
}

export interface IWorkplanPort {
  createPlan(requirements: string[], lang: Language): Promise<Workplan>;
  executePlan(plan: Workplan): AsyncGenerator<StepResult>;
}

export interface ISummaryPort {
  summarizeFile(filePath: string, level: ASTSummary['level']): Promise<ASTSummary>;
  summarizeProject(rootPath: string, level: ASTSummary['level']): Promise<ASTSummary[]>;
}

// ─── Output Ports (Secondary / Driven) ───────────────────

export interface IASTPort {
  extractSummary(filePath: string, level: ASTSummary['level']): Promise<ASTSummary>;
  diffStructural(before: ASTSummary, after: ASTSummary): StructuralDiff;
}

export interface ILLMPort {
  prompt(budget: TokenBudget, messages: Message[]): Promise<LLMResponse>;
  streamPrompt(budget: TokenBudget, messages: Message[]): AsyncGenerator<string>;
}

export interface IBuildPort {
  compile(project: Project): Promise<BuildResult>;
  lint(project: Project): Promise<LintResult>;
  test(project: Project, suite: TestSuite): Promise<TestResult>;
}

export interface IWorktreePort {
  create(branchName: string): Promise<WorktreePath>;
  merge(worktree: WorktreePath, target: string): Promise<MergeResult>;
  cleanup(worktree: WorktreePath): Promise<void>;
  list(): Promise<WorktreePath[]>;
}

export interface IGitPort {
  commit(message: string): Promise<string>;
  createBranch(name: string): Promise<void>;
  diff(base: string, head: string): Promise<string>;
  currentBranch(): Promise<string>;
  /** Return raw git status entries for hygiene analysis */
  statusEntries(): Promise<GitStatusEntry[]>;
  /** List all git worktrees with staleness info */
  worktreeEntries(): Promise<GitWorktreeEntry[]>;
  /** Find embedded .git directories (not the root repo) */
  findEmbeddedRepos(rootPath: string): Promise<string[]>;
}

export interface GitStatusEntry {
  /** Two-char status code, e.g. ' M', 'M ', '??' */
  code: string;
  path: string;
}

export interface GitWorktreeEntry {
  path: string;
  branch: string;
  commit: string;
  /** True if branch has commits after diverging from HEAD of main worktree */
  hasRecentCommits: boolean;
}

export interface IFileSystemPort {
  read(filePath: string): Promise<string>;
  write(filePath: string, content: string): Promise<void>;
  exists(filePath: string): Promise<boolean>;
  glob(pattern: string): Promise<string[]>;
  /** Return file modification time as epoch ms. Returns 0 if unavailable. */
  mtime(filePath: string): Promise<number>;
}

// ─── Validation Ports ────────────────────────────────────

export type {
  BehavioralSpec,
  BehavioralAssertion,
  PropertySpec,
  SmokeScenario,
  SmokeStep,
  SignConvention,
  ValidationVerdict,
  IValidationPort,
} from './validation.js';

// ─── Report Formatting (re-exported from domain) ────────
export { formatArchReport, formatCompactSummary } from '../domain/report-formatter.js';

// ─── Action Items (re-exported from domain) ─────────────
export type { ActionItem, ActionItemReport, ActionPriority, ActionCategory } from '../domain/action-items.js';
export { extractArchActions, extractValidationActions, buildActionItemReport, formatActionItems } from '../domain/action-items.js';

// ─── Agent Executor Ports ────────────────────────────────

export type { IAgentExecutorPort } from './agent-executor.js';
export type {
  AgentTask,
  AgentResult,
  AgentMetrics,
  AgentResultStatus,
  AgentToolCall,
  AgentToolResult,
  AgentContext,
  AgentContextSource,
  ExecutorBackend,
  ComparisonEntry,
  ComparisonReport,
} from '../domain/agent-executor-types.js';

// ─── Hub Command Ports (Bidirectional Hub ↔ Project) ───

export type {
  HubCommandType,
  HubCommandPayload,
  SpawnAgentPayload,
  TerminateAgentPayload,
  CreateTaskPayload,
  CancelTaskPayload,
  RunAnalyzePayload,
  RunBuildPayload,
  RunValidatePayload,
  RunGeneratePayload,
  RunSummarizePayload,
  HubCommand,
  HubCommandStatus,
  HubCommandResult,
  HubCommandHandler,
  IHubCommandReceiverPort,
  IHubCommandSenderPort,
} from './hub-command.js';

// ─── Checkpoint Ports ───────────────────────────────────

export type { CheckpointEntry, TaskSnapshot, FeatureProgress, FeaturePhase, ICheckpointPort } from './checkpoint.js';

// ─── Coordination Ports ─────────────────────────────────

export type {
  ICoordinationPort,
  UnstagedFile,
  WorktreeLock,
  LockResult,
  TaskClaim,
  ClaimResult,
  ActivityEntry,
  InstanceInfo,
  UnstagedState,
} from './coordination.js';

// ─── ADR Ports ─────────────────────────────────────────

export type { IADRPort, IADRQueryPort } from './adr.js';
export type { ADREntry, ADRSnapshot, ADRAbandonedReport, ADRStatus } from '../domain/adr-types.js';

// ─── Secrets Ports ──────────────────────────────────────

export type { SecretContext, SecretMetadata, SecretResult, ISecretsPort } from './secrets.js';

// ─── Analysis Ports ──────────────────────────────────────

export interface IArchAnalysisPort {
  /** Build the full import/export dependency graph from L1 summaries */
  buildDependencyGraph(rootPath: string): Promise<ImportEdge[]>;

  /** Find exports that no other file imports */
  findDeadExports(rootPath: string): Promise<DeadExport[]>;

  /** Validate hexagonal dependency direction rules */
  validateHexBoundaries(rootPath: string): Promise<DependencyViolation[]>;

  /** Detect circular import chains */
  detectCircularDeps(rootPath: string): Promise<string[][]>;

  /** Full analysis: dead code + hex validation + circular detection */
  analyzeArchitecture(rootPath: string): Promise<ArchAnalysisResult>;
}

// ─── Version Ports ──────────────────────────────────────

export interface VersionInfo {
  cli: Version;
  hub: Version | null;
  hubBinaryPath: string | null;
  mismatch: boolean;
}

export interface IVersionPort {
  /** CLI version from package.json */
  getCliVersion(): Version;
  /** hex-hub binary version (null if not installed) */
  getHubVersion(): Promise<Version | null>;
  /** Combined version info with mismatch detection */
  getVersionInfo(): Promise<VersionInfo>;
}
