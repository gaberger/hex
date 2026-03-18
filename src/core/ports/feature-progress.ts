/**
 * Feature Progress Port
 *
 * Tracks feature development progress across the 7-phase hex workflow.
 * Collects structured agent updates and builds ProgressReport views.
 * Used by feature-developer agent to provide clear UX during multi-agent work.
 */

import type { ProgressReport, AgentProgress, Blocker } from './notification.js';
import type { Workplan as DomainWorkplan, WorkplanStep as DomainWorkplanStep } from '../domain/value-objects.js';

// ─── Feature Phases ──────────────────────────────────────────

export type FeaturePhase =
  | 'init'        // HexFlo swarm initialization
  | 'specs'       // Behavioral spec writing (MANDATORY)
  | 'plan'        // Workplan decomposition
  | 'worktrees'   // Git worktree creation
  | 'tier-0'      // Domain + Ports (no dependencies)
  | 'tier-1'      // Secondary adapters (parallel)
  | 'tier-2'      // Primary adapters (parallel)
  | 'tier-3'      // Use cases + composition root
  | 'validate'    // Validation judge (BLOCKING gate)
  | 'integrate'   // Merge worktrees + integration tests
  | 'finalize';   // Cleanup + report

// ─── Workplan Types (extended from domain) ───────────────────

export interface FeatureWorkplan extends DomainWorkplan {
  featureName: string;
  steps: FeatureWorkplanStep[];
  estimatedTotalTokens: number;
  createdAt: number;
}

export interface FeatureWorkplanStep extends DomainWorkplanStep {
  port: string;
  tier: number;           // 0=domain/ports, 1=secondary, 2=primary, 3=usecases
  worktreeBranch: string;
  assignee: 'hex-coder' | 'integrator' | 'planner' | 'behavioral-spec-writer' | 'validation-judge';
  estimatedTokens: number;
  doneCondition: string;
}

// ─── Agent Status Updates ────────────────────────────────────

export interface AgentStatusUpdate {
  agentName: string;
  adapter: string;
  status: 'queued' | 'running' | 'blocked' | 'done' | 'failed';
  currentStep: string; // 'red' | 'green' | 'refactor' | 'lint' | 'test' | 'commit'
  qualityScore?: number;
  iteration: number;
  maxIterations: number;
  error?: string;
  blockedBy?: string; // e.g., "Waiting for domain-changes to merge"
}

// ─── Feature Session ─────────────────────────────────────────

export interface FeatureSession {
  featureName: string;
  workplan: FeatureWorkplan | null; // Null until plan phase completes
  phases: FeaturePhaseStatus[];
  currentPhase: FeaturePhase;
  startedAt: number;
  tokenBudget: number;
  tokenUsed: number;
  agents: Map<string, AgentProgress>; // agentName → current status
}

export interface FeaturePhaseStatus {
  phase: FeaturePhase;
  status: 'pending' | 'in-progress' | 'done' | 'failed';
  startedAt?: number;
  completedAt?: number;
  output?: string; // Path to phase output (e.g., specs file, workplan file)
}

// ─── Feature Report (Final) ──────────────────────────────────

export interface FeatureReport {
  featureName: string;
  verdict: 'PASS' | 'FAIL';
  phases: FeaturePhaseStatus[];
  tasksCompleted: number;
  tasksFailed: number;
  worktreesCreated: number;
  filesChanged: string[];
  testsAdded: number;
  integrationCommit: string;
  tokenUsed: number;
  durationSeconds: number;
  errorSummary?: string;
}

// ─── Port Interface ──────────────────────────────────────────

export interface IFeatureProgressPort {
  /**
   * Start tracking a feature. Initializes session state.
   * Call this in the "init" phase AFTER HexFlo swarm_init.
   */
  startFeature(featureName: string, tokenBudget?: number): Promise<FeatureSession>;

  /**
   * Update agent status. Called by agents via event bus.
   * Rebuilds ProgressReport and notifies listeners.
   */
  updateAgent(update: AgentStatusUpdate): Promise<void>;

  /**
   * Get the current progress report (for display rendering).
   */
  getProgress(): Promise<ProgressReport>;

  /**
   * Load and store the workplan (after plan phase completes).
   * Extracts tier structure and task dependencies.
   */
  loadWorkplan(workplanPath: string): Promise<void>;

  /**
   * Mark a phase as complete and transition to the next.
   * Automatically triggers phase-specific setup (e.g., worktree creation).
   */
  completePhase(phase: FeaturePhase, output?: string): Promise<void>;

  /**
   * Add a blocker (compile error, merge conflict, etc.).
   * Surfaces in ProgressReport.blockers for immediate visibility.
   */
  addBlocker(blocker: Omit<Blocker, 'since'>): Promise<void>;

  /**
   * Remove a blocker once resolved.
   */
  removeBlocker(agentName: string): Promise<void>;

  /**
   * Stop tracking and generate final report.
   * Call this in the "finalize" phase.
   */
  endFeature(verdict: 'PASS' | 'FAIL', commitHash?: string): Promise<FeatureReport>;

  /**
   * Subscribe to progress updates (for CLI display).
   * Callback is invoked every time ProgressReport changes.
   */
  onProgress(callback: (report: ProgressReport) => void): void;

  /**
   * Get the current feature session (or null if no active feature).
   */
  getCurrentSession(): FeatureSession | null;
}
