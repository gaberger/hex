/**
 * Notification Orchestrator Use Case
 *
 * Bridges domain events to the notification system. Listens to DomainEvent
 * emissions, transforms them into Notification objects, tracks aggregate
 * progress across all agents, detects blockers/stalls, and triggers
 * decision requests when an agent hits a fork.
 *
 * Implements smart rate limiting so fast feedback-loop iterations don't
 * flood the terminal with trace-level noise.
 */

import type { DomainEvent } from '../domain/entities.js';
import type {
  Notification,
  NotificationLevel,
  NotificationSource,
  NotificationPreferences,
  ProgressReport,
  AgentProgress,
  Blocker,
  DecisionRequest,
  DecisionResponse,
  INotificationEmitPort,
  INotificationQueryPort,
} from '../ports/notification.js';

// ─── Configuration ──────────────────────────────────────

export interface OrchestratorConfig {
  /** ms with no events before a stall warning fires */
  stallThresholdMs: number;
  /** Number of consecutive quality drops before escalating to a decision */
  convergenceDropLimit: number;
  /** Minimum ms between trace-level notifications (rate limiting) */
  traceThrottleMs: number;
  /** Minimum ms between progress report emissions */
  progressIntervalMs: number;
}

const DEFAULT_CONFIG: OrchestratorConfig = {
  stallThresholdMs: 60_000,
  convergenceDropLimit: 2,
  traceThrottleMs: 2_000,
  progressIntervalMs: 5_000,
};

// ─── Agent Tracking State ───────────────────────────────

interface TrackedAgent {
  progress: AgentProgress;
  qualityHistory: number[];
  lastEventTimestamp: number;
  stallWarningEmitted: boolean;
}

// ─── Orchestrator ───────────────────────────────────────

export class NotificationOrchestrator implements INotificationQueryPort {
  private readonly agents = new Map<string, TrackedAgent>();
  private readonly notifications: Notification[] = [];
  private readonly pendingDecisions = new Map<string, DecisionRequest>();
  private readonly config: OrchestratorConfig;
  private preferences: NotificationPreferences;

  private lastTraceTimestamp = 0;
  private lastProgressTimestamp = 0;
  private nextNotificationId = 1;
  private stallCheckTimer: ReturnType<typeof setInterval> | null = null;

  private swarmId = 'default';
  private currentPhase = 'execute';

  constructor(
    private readonly emitPort: INotificationEmitPort,
    config: Partial<OrchestratorConfig> = {},
    preferences?: Partial<NotificationPreferences>,
  ) {
    this.config = { ...DEFAULT_CONFIG, ...config };
    this.preferences = {
      channels: ['terminal', 'event_bus'],
      minLevel: 'info',
      quietMode: false,
      progressInterval: this.config.progressIntervalMs,
      decisionTimeout: 30_000,
      groupByAdapter: true,
      showTokenUsage: true,
      ...preferences,
    };
  }

  // ─── Lifecycle ──────────────────────────────────────

  start(swarmId: string, phase: string): void {
    this.swarmId = swarmId;
    this.currentPhase = phase;
    this.stallCheckTimer = setInterval(
      () => this.checkForStalls(),
      Math.max(5_000, this.config.stallThresholdMs / 4),
    );
  }

  stop(): void {
    if (this.stallCheckTimer) {
      clearInterval(this.stallCheckTimer);
      this.stallCheckTimer = null;
    }
  }

  // ─── Agent Registration ─────────────────────────────

  registerAgent(agentName: string, adapter: string, maxIterations = 5): void {
    this.agents.set(agentName, {
      progress: {
        agentName,
        adapter,
        status: 'queued',
        currentStep: 'waiting',
        iteration: 0,
        maxIterations,
        lastActivity: Date.now(),
      },
      qualityHistory: [],
      lastEventTimestamp: Date.now(),
      stallWarningEmitted: false,
    });
  }

  // ─── Domain Event Handling ──────────────────────────

  async handleEvent(
    event: DomainEvent,
    source: NotificationSource,
  ): Promise<void> {
    const tracked = this.agents.get(source.agentName);
    if (tracked) {
      tracked.lastEventTimestamp = Date.now();
      tracked.stallWarningEmitted = false;
      tracked.progress.lastActivity = Date.now();
    }

    const mapped = this.mapEventToNotification(event, source, tracked);

    if (this.shouldEmit(mapped.level)) {
      await this.emit(mapped);
    }

    this.updateAgentProgress(event, source, tracked);
    await this.checkConvergence(source.agentName, tracked);
    await this.maybeEmitProgress();
  }

  // ─── Event → Notification Mapping ──────────────────

  private mapEventToNotification(
    event: DomainEvent,
    source: NotificationSource,
    tracked: TrackedAgent | undefined,
  ): Omit<Notification, 'id' | 'timestamp'> {
    const iteration = tracked?.progress.iteration ?? 0;

    switch (event.type) {
      case 'CodeGenerated':
        return {
          level: 'info',
          source,
          title: `Code generated: ${event.payload.filePath}`,
          detail: `Language: ${event.payload.language}, tokens: ${event.payload.tokenCount}`,
          context: { tokensUsed: event.payload.tokenCount },
        };

      case 'LintPassed':
        return {
          level: 'success',
          source,
          title: `Lint passed: ${event.payload.filePath}`,
          detail: event.payload.warningCount > 0
            ? `${event.payload.warningCount} warnings remaining`
            : 'Clean',
        };

      case 'LintFailed':
        return {
          level: iteration < 3 ? 'warning' : 'error',
          source,
          title: `Lint failed: ${event.payload.filePath}`,
          detail: `${event.payload.errors.length} errors (iteration ${iteration})`,
          context: { iterationCount: iteration },
        };

      case 'TestsPassed':
        return {
          level: this.isAdapterComplete(source.adapter)
            ? 'milestone'
            : 'success',
          source,
          title: `Tests passed: ${event.payload.suite}`,
          detail: `${event.payload.passed} passed in ${event.payload.duration}ms`,
        };

      case 'TestsFailed': {
        const repeated = this.isRepeatedTestFailure(source.agentName, event);
        return {
          level: repeated ? 'error' : 'warning',
          source,
          title: `Tests failed: ${event.payload.suite}`,
          detail: `${event.payload.failures.length} failures`,
        };
      }

      case 'BuildSucceeded':
        return {
          level: 'info',
          source,
          title: 'Build succeeded',
          detail: `Completed in ${event.payload.duration}ms`,
        };

      case 'BuildFailed':
        return {
          level: 'error',
          source,
          title: 'Build failed',
          detail: event.payload.errors.join('; ').slice(0, 200),
        };

      case 'WorkplanCreated':
        return {
          level: 'milestone',
          source,
          title: `Workplan created: ${event.payload.planId}`,
          detail: `${event.payload.stepCount} steps`,
          context: { stepsTotal: event.payload.stepCount },
        };

      case 'StepCompleted':
        return {
          level: this.isLastStep(event.payload.stepId) ? 'milestone' : 'info',
          source,
          title: `Step completed: ${event.payload.stepId}`,
          detail: `Status: ${event.payload.status}`,
        };

      case 'SwarmSpawned':
        return {
          level: 'milestone',
          source,
          title: `Swarm spawned: ${event.payload.agentCount} agents`,
          detail: `Topology: ${event.payload.topology}`,
        };
      default:
        return {
          level: 'trace' as const,
          source,
          title: `Event: ${(event as DomainEvent).type}`,
        };
    }
  }

  // ─── Progress Tracking ─────────────────────────────

  private updateAgentProgress(
    event: DomainEvent,
    _source: NotificationSource,
    tracked: TrackedAgent | undefined,
  ): void {
    if (!tracked) return;

    const progress = tracked.progress;
    progress.status = 'running';

    switch (event.type) {
      case 'CodeGenerated':
        progress.currentStep = 'gen';
        break;
      case 'LintPassed':
      case 'LintFailed':
        progress.currentStep = 'lint';
        break;
      case 'TestsPassed':
      case 'TestsFailed':
        progress.currentStep = 'test';
        progress.iteration += 1;
        break;
      case 'BuildSucceeded':
      case 'BuildFailed':
        progress.currentStep = 'build';
        break;
      case 'StepCompleted':
        if (event.payload.status === 'passed') {
          const p = progress as unknown as Record<string, unknown>;
          p['stepsCompleted'] = ((p['stepsCompleted'] as number) ?? 0) + 1;
        }
        break;
    }

    // Track quality score when tests pass or fail
    if (event.type === 'TestsPassed') {
      tracked.qualityHistory.push(100);
    } else if (event.type === 'TestsFailed') {
      const ratio = 1 - event.payload.failures.length * 10;
      tracked.qualityHistory.push(Math.max(0, Math.round(ratio * 100)));
    }
  }

  private computeOverallPercent(): number {
    if (this.agents.size === 0) return 0;

    let total = 0;
    for (const tracked of this.agents.values()) {
      switch (tracked.progress.status) {
        case 'done':
          total += 100;
          break;
        case 'failed':
          total += 0;
          break;
        case 'queued':
          total += 0;
          break;
        case 'running':
        case 'blocked':
          total += Math.min(
            90,
            (tracked.progress.iteration / tracked.progress.maxIterations) * 90,
          );
          break;
      }
    }
    return Math.round(total / this.agents.size);
  }

  // ─── Convergence & Stall Detection ─────────────────

  private async checkConvergence(
    agentName: string,
    tracked: TrackedAgent | undefined,
  ): Promise<void> {
    if (!tracked) return;

    const history = tracked.qualityHistory;
    if (history.length < this.config.convergenceDropLimit + 1) return;

    const recentScores = history.slice(-this.config.convergenceDropLimit - 1);
    let consecutiveDrops = 0;
    for (let i = 1; i < recentScores.length; i++) {
      if (recentScores[i] < recentScores[i - 1]) {
        consecutiveDrops++;
      } else {
        consecutiveDrops = 0;
      }
    }

    if (consecutiveDrops >= this.config.convergenceDropLimit) {
      const decision: Omit<DecisionRequest, 'id'> = {
        agentName,
        question:
          `Quality score declining for ${consecutiveDrops} consecutive iterations. ` +
          `Recent scores: ${recentScores.join(' → ')}. How to proceed?`,
        options: [
          {
            id: 'continue',
            label: 'Continue iterating',
            description: 'Let the agent try more iterations',
            risk: 'medium',
            estimatedImpact: 'May waste tokens if diverging',
          },
          {
            id: 'escalate',
            label: 'Escalate to Opus',
            description: 'Switch to a stronger model for this adapter',
            risk: 'low',
            estimatedImpact: 'Higher cost, likely better results',
          },
          {
            id: 'reset',
            label: 'Reset and retry',
            description: 'Discard current attempt and start fresh',
            risk: 'high',
            estimatedImpact: 'Loses all iteration progress',
          },
        ],
        deadline: this.preferences.decisionTimeout,
        defaultOption: 'escalate',
        context: `Agent ${agentName} working on ${tracked.progress.adapter}`,
      };

      await this.requestDecision(decision);
    }
  }

  private checkForStalls(): void {
    const now = Date.now();
    for (const [agentName, tracked] of this.agents) {
      if (tracked.progress.status !== 'running') continue;
      if (tracked.stallWarningEmitted) continue;

      const idleMs = now - tracked.lastEventTimestamp;
      if (idleMs > this.config.stallThresholdMs) {
        tracked.stallWarningEmitted = true;
        tracked.progress.status = 'blocked';

        // Fire-and-forget; stall check runs from setInterval
        void this.emit({
          level: 'warning',
          source: {
            agentName,
            agentType: 'monitor',
            phase: this.currentPhase as NotificationSource['phase'],
            adapter: tracked.progress.adapter,
          },
          title: `Agent stalled: ${agentName}`,
          detail: `No events for ${Math.round(idleMs / 1000)}s`,
        });
      }
    }
  }

  // ─── Decision Management ───────────────────────────

  private async requestDecision(
    request: Omit<DecisionRequest, 'id'>,
  ): Promise<void> {
    const response = await this.emitPort.requestDecision(request);

    const tracked = this.agents.get(request.agentName);
    if (tracked && response.selectedOption === 'reset') {
      tracked.qualityHistory = [];
      tracked.progress.iteration = 0;
    }
  }

  // ─── Rate Limiting ─────────────────────────────────

  private shouldEmit(level: NotificationLevel): boolean {
    if (this.preferences.quietMode) {
      return level === 'error' || level === 'decision' || level === 'milestone';
    }

    const levelOrder: NotificationLevel[] = [
      'trace', 'info', 'success', 'warning', 'error', 'decision', 'milestone',
    ];
    const minIdx = levelOrder.indexOf(this.preferences.minLevel);
    const currentIdx = levelOrder.indexOf(level);
    if (currentIdx < minIdx) return false;

    if (level === 'trace') {
      const now = Date.now();
      if (now - this.lastTraceTimestamp < this.config.traceThrottleMs) {
        return false;
      }
      this.lastTraceTimestamp = now;
    }

    return true;
  }

  // ─── Emission Helpers ──────────────────────────────

  private async emit(
    partial: Omit<Notification, 'id' | 'timestamp'>,
  ): Promise<void> {
    const notification: Notification = {
      id: `notif-${this.nextNotificationId++}`,
      timestamp: Date.now(),
      ...partial,
    };
    this.notifications.push(notification);
    await this.emitPort.notify(partial);
  }

  private async maybeEmitProgress(): Promise<void> {
    const now = Date.now();
    if (now - this.lastProgressTimestamp < this.config.progressIntervalMs) {
      return;
    }
    this.lastProgressTimestamp = now;

    const report = await this.getProgress();
    await this.emitPort.reportProgress(report);
  }

  // ─── Helpers ───────────────────────────────────────

  private isAdapterComplete(adapter: string | undefined): boolean {
    if (!adapter) return false;
    for (const tracked of this.agents.values()) {
      if (tracked.progress.adapter === adapter && tracked.progress.status === 'done') {
        return true;
      }
    }
    return false;
  }

  private isRepeatedTestFailure(
    agentName: string,
    _event: Extract<DomainEvent, { type: 'TestsFailed' }>,
  ): boolean {
    const tracked = this.agents.get(agentName);
    if (!tracked) return false;
    // Repeated if agent has had at least one prior test failure
    const failCount = tracked.qualityHistory.filter((q) => q < 100).length;
    return failCount > 0;
  }

  private isLastStep(stepId: string): boolean {
    // Heuristic: step IDs ending with the highest index are last.
    // In production this would check the TaskGraph.
    return stepId.endsWith('-final') || stepId.endsWith('-last');
  }

  markAgentDone(agentName: string, qualityScore?: number): void {
    const tracked = this.agents.get(agentName);
    if (!tracked) return;
    tracked.progress.status = 'done';
    if (qualityScore !== undefined) {
      tracked.progress.qualityScore = qualityScore;
    }
  }

  markAgentFailed(agentName: string): void {
    const tracked = this.agents.get(agentName);
    if (!tracked) return;
    tracked.progress.status = 'failed';
  }

  // ─── INotificationQueryPort ────────────────────────

  async getProgress(): Promise<ProgressReport> {
    const agents = Array.from(this.agents.values()).map((t) => ({
      ...t.progress,
      qualityScore: t.qualityHistory.length > 0
        ? t.qualityHistory[t.qualityHistory.length - 1]
        : undefined,
    }));

    const blockers: Blocker[] = [];
    const now = Date.now();

    for (const tracked of this.agents.values()) {
      if (tracked.progress.status === 'blocked') {
        blockers.push({
          agentName: tracked.progress.agentName,
          type: 'timeout',
          description: `No activity for ${Math.round((now - tracked.lastEventTimestamp) / 1000)}s`,
          since: tracked.lastEventTimestamp,
          suggestedAction: 'Check agent health or restart',
        });
      }
    }

    return {
      swarmId: this.swarmId,
      phase: this.currentPhase,
      agents,
      overallPercent: this.computeOverallPercent(),
      blockers,
    };
  }

  async getPendingDecisions(): Promise<DecisionRequest[]> {
    return Array.from(this.pendingDecisions.values());
  }

  async respondToDecision(response: DecisionResponse): Promise<void> {
    this.pendingDecisions.delete(response.requestId);
  }

  async getRecent(
    limit: number,
    minLevel?: NotificationLevel,
  ): Promise<Notification[]> {
    let filtered = this.notifications;
    if (minLevel) {
      const levelOrder: NotificationLevel[] = [
        'trace', 'info', 'success', 'warning', 'error', 'decision', 'milestone',
      ];
      const minIdx = levelOrder.indexOf(minLevel);
      filtered = filtered.filter(
        (n) => levelOrder.indexOf(n.level) >= minIdx,
      );
    }
    return filtered.slice(-limit);
  }

  async setPreferences(
    prefs: Partial<NotificationPreferences>,
  ): Promise<void> {
    this.preferences = { ...this.preferences, ...prefs };
  }
}
