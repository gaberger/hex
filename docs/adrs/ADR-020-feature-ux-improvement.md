# ADR-020: Feature Development UX Improvement

## Status: Accepted
## Date: 2026-03-17
**Deciders**: Core Team
**Related**: ADR-011 (Coordination), ADR-014 (No mock.module), ADR-015 (Hub Persistence)

## Context

The current `/hex-feature-dev` workflow is **noisy and unclear**:

### Current Problems

1. **Agent Chatter Floods Console**
   - Each spawned agent logs full tool calls (Read, Write, Bash)
   - Multiplied across 8 parallel agents = overwhelming noise
   - User loses track of overall progress

2. **No Clear Progress View**
   - Can't tell which phase (specs/plan/code/validate/integrate)
   - Can't see which tasks are done vs. in-progress vs. blocked
   - No visibility into worktree status

3. **Workplan Hidden**
   - Planner creates `docs/workplans/feat-<name>.json`
   - User never sees it formatted
   - Dependency graph not visualized

4. **Architecture Boundaries Unclear**
   - Tasks span domain/ports/adapters but user doesn't see the decomposition
   - Hex layer violations not surfaced until `hex analyze` runs

5. **Status Updates Scattered**
   - Ruflo task updates mixed with agent logs
   - No unified status line like dashboard has

### User Experience Gap

**What the user sees now:**
```
[planner] Reading src/core/ports/index.ts...
[planner] Tool: Read { file_path: "..." }
[hex-coder-1] Spawned for adapter: git-adapter
[hex-coder-1] Tool: Bash { command: "cd ../hex-feat-webhook-git && bun test" }
[hex-coder-2] Spawned for adapter: cli-adapter
[ruflo] Task created: implement-git-adapter
... (hundreds of lines of tool calls)
```

**What the user SHOULD see:**
```
hex feature: webhook-notifications
────────────────────────────────────────────────────────────────────────
Phase 1/7: SPECS     ✓ Complete (5 specs, 1 negative)
Phase 2/7: PLAN      ✓ Complete (8 tasks, 3 tiers)
Phase 3/7: WORKTREES ✓ Created 8 worktrees
Phase 4/7: CODE      ⟳ In Progress (3/8 done, 5 running)

Workplan:
  Tier 0 (domain/ports)
    ✓ domain-changes       (feat/webhook-notifications/domain)
    ✓ port-changes         (feat/webhook-notifications/ports)

  Tier 1 (adapters - parallel)
    ✓ git-adapter          Q:95  [===========] test
    ⟳ webhook-adapter      Q:82  [========---] lint
    ⟳ cli-adapter          Q:78  [=======----] test
    ⏳ mcp-adapter                [           ] queued
    ⏳ fs-adapter                 [           ] queued

  Tier 2 (integration)
    ⏳ composition-root          [           ] queued
    ⏳ integration-tests         [           ] queued

Overall: 38% │ Tokens: 124k/500k │ Time: 3m42s │ Blockers: 0
────────────────────────────────────────────────────────────────────────
[Press 'd' for details | 'q' to abort | 'h' for help]
```

## Decision

Implement a **Feature Progress Orchestrator** that:

1. **Collects structured events** from agents (not raw logs)
2. **Formats progress reports** using existing status-formatter
3. **Displays a persistent status view** in the CLI
4. **Surfaces the workplan** as a visual tree
5. **Shows hex architecture boundaries** (domain → ports → adapters)

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│  CLI (Primary Adapter)                                      │
│  ┌──────────────────────────────────────────────────────┐  │
│  │  Feature Progress Display                            │  │
│  │  - Persistent status line (refreshes in-place)      │  │
│  │  - Workplan tree view                                │  │
│  │  - Agent progress bars                               │  │
│  │  - Interactive controls (d/q/h)                      │  │
│  └──────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
                            ↕
┌─────────────────────────────────────────────────────────────┐
│  Feature Progress Orchestrator (Use Case)                   │
│  - Aggregates agent status updates                          │
│  - Builds ProgressReport from workplan + ruflo tasks        │
│  - Emits StatusLine updates via INotificationEmitPort       │
│  - Manages phase transitions (specs → plan → code → ...)    │
└─────────────────────────────────────────────────────────────┘
                            ↕
┌─────────────────────────────────────────────────────────────┐
│  Event Bus (Secondary Adapter)                              │
│  - Agents publish: AgentStatusEvent (no logs, just state)  │
│  - Orchestrator subscribes to: agent.* topics               │
│  - Formats: { agentId, phase, status, task, iteration }    │
└─────────────────────────────────────────────────────────────┘
```

### Key Changes

#### 1. **New Port: `IFeatureProgressPort`**

```typescript
// src/core/ports/feature-progress.ts
export interface IFeatureProgressPort {
  /** Start tracking a feature (loads workplan + creates ProgressReport) */
  startFeature(featureName: string): Promise<FeatureSession>;

  /** Update agent status (called by agents, not ruflo) */
  updateAgent(update: AgentStatusUpdate): Promise<void>;

  /** Get current progress report */
  getProgress(): Promise<ProgressReport>;

  /** Mark phase complete and transition to next */
  completePhase(phase: FeaturePhase): Promise<void>;

  /** Stop tracking and cleanup */
  endFeature(): Promise<FeatureReport>;
}

export interface FeatureSession {
  featureName: string;
  workplan: Workplan;
  phases: FeaturePhase[];
  currentPhase: FeaturePhase;
  startedAt: number;
}

export type FeaturePhase =
  | 'specs' | 'plan' | 'worktrees'
  | 'tier-0' | 'tier-1' | 'tier-2'
  | 'validate' | 'integrate';

export interface AgentStatusUpdate {
  agentName: string;
  adapter: string;
  status: 'running' | 'blocked' | 'done' | 'failed';
  currentStep: string; // 'red' | 'green' | 'refactor' | 'lint' | 'test'
  qualityScore?: number;
  iteration: number;
  error?: string;
}
```

#### 2. **Update feature-developer Agent**

Modify `agents/feature-developer.yml`:

```yaml
workflow:
  phases:
    - id: init
      steps:
        - id: start-progress-tracking
          action: |
            # Initialize feature progress orchestrator
            ctx.featureProgress.startFeature("{{feature_name}}")
```

Each agent spawn includes a progress callback:

```yaml
    - id: tier-1-2
      agents:
        - type: hex-coder
          mode: bypassPermissions
          run_in_background: true
          on_status_change: |
            # Emit structured event (not logs)
            ctx.eventBus.publish('agent.status', {
              agentName: "{{agent_name}}",
              adapter: "{{adapter}}",
              status: "{{status}}",
              currentStep: "{{step}}",
              qualityScore: {{quality}},
              iteration: {{iter}}
            })
```

#### 3. **CLI Progress Display**

```typescript
// src/adapters/primary/feature-progress-display.ts
export class FeatureProgressDisplay {
  private lastReport: ProgressReport | null = null;
  private clearLines = 0;

  async start(featureName: string): Promise<void> {
    // Clear screen and show initial state
    this.clear();
    this.render(await this.getInitialReport(featureName));

    // Subscribe to progress updates
    this.ctx.featureProgress.onProgress((report) => {
      this.clear();
      this.render(report);
    });

    // Setup keyboard handler
    process.stdin.setRawMode(true);
    process.stdin.on('data', (key) => this.handleKey(key));
  }

  private render(report: ProgressReport): void {
    const lines = [
      this.renderHeader(report),
      this.renderPhases(report),
      this.renderWorkplan(report),
      this.renderSummary(report),
      this.renderFooter(),
    ].flat();

    console.log(lines.join('\n'));
    this.clearLines = lines.length;
  }

  private clear(): void {
    // Move cursor up N lines and clear
    if (this.clearLines > 0) {
      process.stdout.write(`\x1b[${this.clearLines}A\x1b[J`);
    }
  }

  private renderWorkplan(report: ProgressReport): string[] {
    // Tree view with indent, icons, progress bars
    const lines = ['Workplan:'];
    for (const tier of report.tiers) {
      lines.push(`  Tier ${tier.level} (${tier.layer})`);
      for (const task of tier.tasks) {
        const icon = STATUS_ICON_ANSI[task.status];
        const bar = progressBarAnsi(task.percent, 12);
        const name = padRight(task.adapter, 20);
        const quality = task.qualityScore ? `Q:${task.qualityScore}` : '';
        lines.push(`    ${icon} ${name} ${bar} ${quality}`);
      }
      lines.push('');
    }
    return lines;
  }

  private handleKey(key: Buffer): void {
    const char = key.toString();
    if (char === 'd') this.showDetails();
    if (char === 'q') this.abort();
    if (char === 'h') this.showHelp();
  }
}
```

#### 4. **Agent Stdout Redirection**

Agents spawned with `run_in_background: true` should:
- **NOT** log to parent stdout
- **EMIT** structured events to event bus
- **LOG** verbose details to `.hex/logs/agent-<name>.log`

```typescript
// src/core/usecases/agent-executor.ts
async spawnBackgroundAgent(config: AgentConfig): Promise<Agent> {
  const logPath = `.hex/logs/agent-${config.name}.log`;
  const logStream = createWriteStream(logPath, { flags: 'a' });

  const agent = await this.agentTool.spawn({
    ...config,
    stdout: logStream, // Redirect to file
    stderr: logStream,
    onStatusChange: (status) => {
      // Emit structured event
      this.eventBus.publish('agent.status', {
        agentName: config.name,
        adapter: config.adapter,
        status: status.status,
        currentStep: status.step,
        qualityScore: status.quality,
        iteration: status.iteration,
      });
    },
  });

  return agent;
}
```

#### 5. **Workplan Visualizer**

```typescript
// src/core/usecases/workplan-visualizer.ts
export class WorkplanVisualizer {
  formatTree(workplan: Workplan): string[] {
    const tiers = this.groupByTier(workplan.steps);
    const lines: string[] = [];

    lines.push(`Feature: ${workplan.title}`);
    lines.push(`Tasks: ${workplan.steps.length} │ Tiers: ${tiers.length}`);
    lines.push('');

    for (const tier of tiers) {
      lines.push(`Tier ${tier.level}: ${tier.label}`);
      lines.push(`  Dependencies: ${tier.dependencies.join(', ') || 'none'}`);
      lines.push(`  Parallelizable: ${tier.tasks.length} tasks`);
      lines.push('');

      for (const task of tier.tasks) {
        const arrow = '  →';
        lines.push(`  ${arrow} ${task.adapter}`);
        lines.push(`      Port: ${task.port}`);
        lines.push(`      Branch: ${task.worktree_branch}`);
        lines.push(`      Estimate: ${task.estimated_tokens} tokens`);
        lines.push('');
      }
    }

    return lines;
  }

  private groupByTier(steps: WorkplanStep[]): Tier[] {
    // Group by dependency level
    const tiers: Map<number, Tier> = new Map();

    for (const step of steps) {
      const level = this.calculateTierLevel(step, steps);
      if (!tiers.has(level)) {
        tiers.set(level, {
          level,
          label: this.tierLabel(level),
          dependencies: this.tierDependencies(level),
          tasks: [],
        });
      }
      tiers.get(level)!.tasks.push(step);
    }

    return Array.from(tiers.values()).sort((a, b) => a.level - b.level);
  }

  private tierLabel(level: number): string {
    const labels = [
      'Domain & Ports',
      'Secondary Adapters',
      'Primary Adapters',
      'Use Cases',
      'Composition Root',
      'Integration Tests',
    ];
    return labels[level] ?? `Tier ${level}`;
  }
}
```

## Consequences

### Positive

1. **Cleaner Console Output**
   - Single status line instead of hundreds of log lines
   - User sees progress at a glance

2. **Better Situational Awareness**
   - Clear phases: know where you are in the workflow
   - Workplan visibility: understand the decomposition
   - Architecture boundaries: see hex layer separation

3. **Actionable Blockers**
   - Failed tasks surfaced immediately
   - Quality scores visible (triggers iteration)
   - Merge conflicts detected early

4. **Interactive Controls**
   - Press 'd' for detailed agent logs
   - Press 'q' to abort cleanly
   - Press 'h' for help

5. **Reusable Components**
   - `IFeatureProgressPort` can be used by other workflows
   - `WorkplanVisualizer` useful for planning-only mode
   - `status-formatter` already handles display logic

### Negative

1. **More Complexity**
   - New port + adapter + use case
   - Event bus subscription management
   - Background agent stdout redirection

2. **Breaking Change for Agents**
   - Existing agents must be updated to emit structured events
   - Old-style verbose agents will be noisy until migrated

3. **Terminal Compatibility**
   - ANSI escape codes may not work in all terminals
   - Fallback to plain text needed for CI/CD

4. **State Management**
   - Progress orchestrator must reconcile ruflo task state + agent events
   - Risk of desync if events are missed

### Mitigation

- **Gradual Migration**: Start with feature-developer agent only
- **Fallback Mode**: Flag `--verbose` shows old-style full logs
- **Terminal Detection**: Auto-disable ANSI if not TTY
- **Event Replay**: Store events in `.hex/events.jsonl` for recovery

## Implementation Plan

### Phase 1: Core Infrastructure (Week 1)
- [ ] Define `IFeatureProgressPort` interface
- [ ] Implement `FeatureProgressOrchestrator` use case
- [ ] Add event bus subscriptions for `agent.status`
- [ ] Create `.hex/logs/` directory for agent logs

### Phase 2: Display Layer (Week 1)
- [ ] Build `FeatureProgressDisplay` CLI adapter
- [ ] Implement workplan tree renderer
- [ ] Add interactive keyboard controls (d/q/h)
- [ ] Terminal compatibility checks

### Phase 3: Agent Integration (Week 2)
- [ ] Update `feature-developer.yml` to use progress port
- [ ] Redirect background agent stdout to log files
- [ ] Emit structured status events from hex-coder
- [ ] Test with real feature (e.g., webhook-notifications)

### Phase 4: Polish (Week 2)
- [ ] Add `--verbose` flag for full logs
- [ ] Store events in `.hex/events.jsonl`
- [ ] Document keyboard shortcuts in help text
- [ ] Update CLAUDE.md with new UX flow

## Alternatives Considered

### 1. **Just Use Dashboard**
- Users would need to open browser to see progress
- Not feasible for CLI-only workflows
- Verdict: Dashboard is supplementary, not replacement

### 2. **Silence Agent Logs Entirely**
- Debugging becomes impossible
- Loses transparency into what agents are doing
- Verdict: Redirect to log files instead

### 3. **Use Ruflo's Status System**
- Ruflo shows "idle" because it's a registry, not executor
- Ruflo doesn't know about hex-specific phases/layers
- Verdict: Hex needs its own domain-specific progress view

## References

- Existing: `src/core/usecases/status-formatter.ts` (progress rendering)
- Existing: `src/core/ports/notification.ts` (status types)
- Existing: `src/adapters/primary/dashboard-adapter.ts` (hub push)
- Related: ADR-011 (Multi-instance coordination)
- Related: ADR-015 (Hub SQLite persistence)

## Success Metrics

- **Reduced Noise**: Console output < 50 lines during feature dev
- **Progress Clarity**: User can answer "what phase am I in?" in < 2 seconds
- **Workplan Visibility**: Tree view shows all tasks + dependencies
- **Error Surfacing**: Blockers visible in status line within 1 update cycle
- **Agent Transparency**: Full logs available via 'd' key or log files
