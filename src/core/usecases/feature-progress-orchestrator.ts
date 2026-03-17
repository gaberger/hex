/**
 * Feature Progress Orchestrator
 *
 * Aggregates agent status updates and builds ProgressReport views.
 * Tracks feature lifecycle across 7 phases with tier-based task grouping.
 * Emits clean progress updates instead of noisy agent logs.
 */

import type {
  IFeatureProgressPort,
  FeatureSession,
  FeaturePhase,
  FeaturePhaseStatus,
  FeatureReport,
  Workplan,
  WorkplanStep,
  AgentStatusUpdate,
} from '../ports/feature-progress.js';
import type {
  ProgressReport,
  AgentProgress,
  Blocker,
} from '../ports/notification.js';
import type { IFileSystemPort } from '../ports/index.js';

export class FeatureProgressOrchestrator implements IFeatureProgressPort {
  private session: FeatureSession | null = null;
  private listeners: Array<(report: ProgressReport) => void> = [];
  private blockers: Blocker[] = [];

  constructor(private readonly fs: IFileSystemPort) {}

  // ─── Session Management ──────────────────────────────────

  async startFeature(
    featureName: string,
    tokenBudget: number = 500_000,
  ): Promise<FeatureSession> {
    if (this.session) {
      throw new Error(
        `Feature "${this.session.featureName}" is already in progress. Call endFeature() first.`,
      );
    }

    const allPhases: FeaturePhase[] = [
      'init',
      'specs',
      'plan',
      'worktrees',
      'tier-0',
      'tier-1',
      'tier-2',
      'tier-3',
      'validate',
      'integrate',
      'finalize',
    ];

    this.session = {
      featureName,
      workplan: null,
      phases: allPhases.map((phase) => ({
        phase,
        status: phase === 'init' ? 'in-progress' : 'pending',
        startedAt: phase === 'init' ? Date.now() : undefined,
      })),
      currentPhase: 'init',
      startedAt: Date.now(),
      tokenBudget,
      tokenUsed: 0,
      agents: new Map(),
    };

    this.emitProgress();
    return this.session;
  }

  async endFeature(
    verdict: 'PASS' | 'FAIL',
    commitHash?: string,
  ): Promise<FeatureReport> {
    if (!this.session) {
      throw new Error('No active feature session');
    }

    const durationSeconds = Math.floor((Date.now() - this.session.startedAt) / 1000);
    const tasksCompleted = Array.from(this.session.agents.values()).filter(
      (a) => a.status === 'done',
    ).length;
    const tasksFailed = Array.from(this.session.agents.values()).filter(
      (a) => a.status === 'failed',
    ).length;

    const report: FeatureReport = {
      featureName: this.session.featureName,
      verdict,
      phases: this.session.phases,
      tasksCompleted,
      tasksFailed,
      worktreesCreated: this.session.workplan?.steps.length ?? 0,
      filesChanged: [], // TODO: extract from git diff
      testsAdded: 0, // TODO: extract from test files
      integrationCommit: commitHash ?? '',
      tokenUsed: this.session.tokenUsed,
      durationSeconds,
      errorSummary: verdict === 'FAIL' ? this.buildErrorSummary() : undefined,
    };

    this.session = null;
    this.blockers = [];
    this.listeners = [];

    return report;
  }

  getCurrentSession(): FeatureSession | null {
    return this.session;
  }

  // ─── Workplan Loading ────────────────────────────────────

  async loadWorkplan(workplanPath: string): Promise<void> {
    if (!this.session) {
      throw new Error('No active feature session');
    }

    const content = await this.fs.readFile(workplanPath);
    const workplan = JSON.parse(content) as Workplan;

    this.session.workplan = workplan;

    // Initialize agent progress for each step
    for (const step of workplan.steps) {
      this.session.agents.set(step.id, {
        agentName: step.id,
        adapter: step.adapter,
        status: 'queued',
        currentStep: 'queued',
        iteration: 0,
        maxIterations: 3,
        lastActivity: Date.now(),
      });
    }

    this.emitProgress();
  }

  // ─── Phase Management ────────────────────────────────────

  async completePhase(phase: FeaturePhase, output?: string): Promise<void> {
    if (!this.session) {
      throw new Error('No active feature session');
    }

    const phaseStatus = this.session.phases.find((p) => p.phase === phase);
    if (!phaseStatus) {
      throw new Error(`Unknown phase: ${phase}`);
    }

    if (phaseStatus.status !== 'in-progress') {
      throw new Error(
        `Phase ${phase} is not in progress (current status: ${phaseStatus.status})`,
      );
    }

    phaseStatus.status = 'done';
    phaseStatus.completedAt = Date.now();
    if (output) {
      phaseStatus.output = output;
    }

    // Auto-transition to next phase
    const currentIndex = this.session.phases.findIndex((p) => p.phase === phase);
    if (currentIndex < this.session.phases.length - 1) {
      const nextPhase = this.session.phases[currentIndex + 1];
      nextPhase.status = 'in-progress';
      nextPhase.startedAt = Date.now();
      this.session.currentPhase = nextPhase.phase;
    }

    this.emitProgress();
  }

  // ─── Agent Status Updates ────────────────────────────────

  async updateAgent(update: AgentStatusUpdate): Promise<void> {
    if (!this.session) {
      throw new Error('No active feature session');
    }

    const existing = this.session.agents.get(update.agentName);
    const agentProgress: AgentProgress = {
      agentName: update.agentName,
      adapter: update.adapter,
      status: update.status,
      currentStep: update.currentStep,
      qualityScore: update.qualityScore,
      iteration: update.iteration,
      maxIterations: update.maxIterations,
      lastActivity: Date.now(),
    };

    this.session.agents.set(update.agentName, agentProgress);

    // Auto-add blocker if status = 'blocked' or 'failed'
    if (update.status === 'blocked' || update.status === 'failed') {
      await this.addBlocker({
        agentName: update.agentName,
        type:
          update.status === 'failed'
            ? update.error?.includes('compile')
              ? 'compile_error'
              : 'test_failure'
            : 'decision_needed',
        description: update.error ?? update.blockedBy ?? 'Unknown blocker',
        suggestedAction: update.status === 'failed' ? 'Check agent logs' : undefined,
      });
    } else if (existing?.status === 'blocked' || existing?.status === 'failed') {
      // Agent recovered — remove blocker
      await this.removeBlocker(update.agentName);
    }

    this.emitProgress();
  }

  // ─── Blocker Management ──────────────────────────────────

  async addBlocker(blocker: Omit<Blocker, 'since'>): Promise<void> {
    // Deduplicate by agentName
    const existing = this.blockers.find((b) => b.agentName === blocker.agentName);
    if (existing) {
      existing.description = blocker.description;
      existing.type = blocker.type;
      existing.suggestedAction = blocker.suggestedAction;
    } else {
      this.blockers.push({ ...blocker, since: Date.now() });
    }
    this.emitProgress();
  }

  async removeBlocker(agentName: string): Promise<void> {
    this.blockers = this.blockers.filter((b) => b.agentName !== agentName);
    this.emitProgress();
  }

  // ─── Progress Report Builder ─────────────────────────────

  async getProgress(): Promise<ProgressReport> {
    if (!this.session) {
      throw new Error('No active feature session');
    }

    const agents = Array.from(this.session.agents.values());
    const doneCount = agents.filter((a) => a.status === 'done').length;
    const totalCount = agents.length;
    const overallPercent = totalCount > 0 ? Math.round((doneCount / totalCount) * 100) : 0;

    return {
      swarmId: `feature-${this.session.featureName}`,
      phase: this.phaseLabel(this.session.currentPhase),
      agents,
      overallPercent,
      blockers: this.blockers,
    };
  }

  // ─── Event Emission ──────────────────────────────────────

  onProgress(callback: (report: ProgressReport) => void): void {
    this.listeners.push(callback);
  }

  private emitProgress(): void {
    if (!this.session) return;

    void this.getProgress().then((report) => {
      for (const listener of this.listeners) {
        listener(report);
      }
    });
  }

  // ─── Helpers ─────────────────────────────────────────────

  private phaseLabel(phase: FeaturePhase): string {
    const labels: Record<FeaturePhase, string> = {
      init: 'Initializing',
      specs: 'Writing Specs',
      plan: 'Planning',
      worktrees: 'Creating Worktrees',
      'tier-0': 'Domain & Ports',
      'tier-1': 'Secondary Adapters',
      'tier-2': 'Primary Adapters',
      'tier-3': 'Use Cases',
      validate: 'Validating',
      integrate: 'Integrating',
      finalize: 'Finalizing',
    };
    return labels[phase];
  }

  private buildErrorSummary(): string {
    const failed = Array.from(this.session?.agents.values() ?? []).filter(
      (a) => a.status === 'failed',
    );
    if (failed.length === 0) return 'Unknown error';

    return failed.map((a) => `${a.adapter}: ${a.currentStep} failed`).join(', ');
  }
}
