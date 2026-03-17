# Feature UX Integration Guide

This guide explains how to integrate the new feature progress system into the hex codebase.

## Overview

The feature progress system consists of:

1. **Port** (`IFeatureProgressPort`) — Contract for progress tracking
2. **Use Case** (`FeatureProgressOrchestrator`) — Business logic for aggregating agent status
3. **Primary Adapter** (`FeatureProgressDisplay`) — CLI rendering with keyboard controls
4. **Agent Updates** — Modify `feature-developer.yml` to emit structured events

## Integration Steps

### Step 1: Wire Dependencies in `composition-root.ts`

```typescript
// src/composition-root.ts

import { FeatureProgressOrchestrator } from './core/usecases/feature-progress-orchestrator.js';
import { FeatureProgressDisplay } from './adapters/primary/feature-progress-display.js';

export async function createAppContext(config: AppConfig): Promise<AppContext> {
  // ... existing adapters ...

  // Feature progress orchestrator
  const featureProgress = new FeatureProgressOrchestrator(fs);

  return {
    // ... existing ports ...
    featureProgress,
  };
}
```

### Step 2: Update `AppContext` Type

```typescript
// src/core/ports/app-context.ts

import type { IFeatureProgressPort } from './feature-progress.js';

export interface AppContext {
  // ... existing ports ...
  featureProgress: IFeatureProgressPort;
}
```

### Step 3: Add CLI Command

```typescript
// src/adapters/primary/cli-adapter.ts

import { FeatureProgressDisplay } from './feature-progress-display.js';

export async function runCLI(ctx: AppContext, args: string[]): Promise<void> {
  const command = args[0];

  if (command === 'feature' && args[1] === 'dev') {
    const featureName = args[2];
    if (!featureName) {
      console.error('Usage: hex feature dev <feature-name>');
      process.exit(1);
    }

    const verbose = args.includes('--verbose');
    const display = new FeatureProgressDisplay(ctx.featureProgress, verbose);

    try {
      await display.start(featureName);
      // Feature workflow runs here (spawns agents, tracks progress)
      // Display updates automatically via onProgress() callbacks
      await waitForFeatureCompletion(ctx, featureName);
    } finally {
      display.stop();
    }

    return;
  }

  // ... existing commands ...
}

async function waitForFeatureCompletion(
  ctx: AppContext,
  featureName: string,
): Promise<void> {
  // Poll session status until finalize phase completes
  while (true) {
    const session = ctx.featureProgress.getCurrentSession();
    if (!session) break;

    const finalizePhase = session.phases.find((p) => p.phase === 'finalize');
    if (finalizePhase?.status === 'done' || finalizePhase?.status === 'failed') {
      break;
    }

    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
}
```

### Step 4: Update `feature-developer.yml` Agent

```yaml
# agents/feature-developer.yml

workflow:
  phases:
    - id: init
      steps:
        - id: start-progress-tracking
          action: |
            await ctx.featureProgress.startFeature("{{feature_name}}", 500_000);

        - id: swarm-init
          action: |
            mcp__ruflo__swarm_init({
              topology: "hierarchical",
              maxAgents: 8,
              strategy: "specialized"
            })

    - id: specs
      agent: behavioral-spec-writer
      on_start: |
        await ctx.featureProgress.completePhase('init');
        await ctx.featureProgress.completePhase('specs'); # Mark as in-progress
      on_complete: |
        await ctx.featureProgress.completePhase('specs', 'docs/specs/{{feature_name}}.json');

    - id: plan
      agent: planner
      on_start: |
        await ctx.featureProgress.completePhase('plan'); # Mark as in-progress
      on_complete: |
        const workplanPath = 'docs/workplans/feat-{{feature_name}}.json';
        await ctx.featureProgress.loadWorkplan(workplanPath);
        await ctx.featureProgress.completePhase('plan', workplanPath);

    - id: tier-0
      agents:
        - type: hex-coder
          scope: domain
          on_status_change: |
            await ctx.featureProgress.updateAgent({
              agentName: "domain-coder",
              adapter: "domain",
              status: "{{status}}",
              currentStep: "{{step}}",
              qualityScore: {{quality}},
              iteration: {{iter}},
              maxIterations: 3,
            });

    - id: tier-1-2
      agents:
        - type: hex-coder
          mode: bypassPermissions
          run_in_background: true
          on_status_change: |
            await ctx.featureProgress.updateAgent({
              agentName: "{{agent_name}}",
              adapter: "{{adapter}}",
              status: "{{status}}",
              currentStep: "{{step}}",
              qualityScore: {{quality}},
              iteration: {{iter}},
              maxIterations: 3,
            });

    - id: validate
      agent: validation-judge
      on_start: |
        await ctx.featureProgress.completePhase('tier-3');
        await ctx.featureProgress.completePhase('validate'); # Mark as in-progress
      on_complete: |
        await ctx.featureProgress.completePhase('validate', 'verdict: {{verdict}}');

    - id: integrate
      agent: integrator
      on_start: |
        await ctx.featureProgress.completePhase('integrate'); # Mark as in-progress
      on_complete: |
        await ctx.featureProgress.completePhase('integrate', 'commit: {{hash}}');

    - id: finalize
      steps:
        - id: cleanup-worktrees
          action: |
            ./scripts/feature-workflow.sh cleanup {{feature_name}}

        - id: generate-report
          action: |
            const verdict = "{{validation_verdict}}";
            const commitHash = "{{integration_commit}}";
            const report = await ctx.featureProgress.endFeature(verdict, commitHash);

            # Write report to file
            const reportPath = `docs/reports/feat-{{feature_name}}-report.json`;
            await ctx.fs.writeFile(reportPath, JSON.stringify(report, null, 2));

            console.log(`\n\nFeature Report: ${reportPath}`);
            console.log(`Verdict: ${verdict}`);
            console.log(`Tasks: ${report.tasksCompleted} completed, ${report.tasksFailed} failed`);
            console.log(`Duration: ${report.durationSeconds}s`);
```

### Step 5: Redirect Agent Logs to Files

```typescript
// src/core/usecases/agent-executor.ts

import { createWriteStream } from 'node:fs';
import { mkdir } from 'node:fs/promises';

export async function spawnBackgroundAgent(
  ctx: AppContext,
  config: AgentConfig,
): Promise<void> {
  // Ensure log directory exists
  await mkdir('.hex/logs', { recursive: true });

  const logPath = `.hex/logs/agent-${config.name}.log`;
  const logStream = createWriteStream(logPath, { flags: 'a' });

  // Spawn agent with stdout/stderr redirected
  await ctx.agent.spawn({
    ...config,
    mode: 'bypassPermissions',
    run_in_background: true,
    // Redirect logs to file (not parent stdout)
    onStdout: (line: string) => {
      logStream.write(`[${new Date().toISOString()}] ${line}\n`);
    },
    onStderr: (line: string) => {
      logStream.write(`[${new Date().toISOString()}] [ERROR] ${line}\n`);
    },
    // Emit structured status updates (not raw logs)
    onStatusChange: async (status: AgentStatus) => {
      await ctx.featureProgress.updateAgent({
        agentName: config.name,
        adapter: config.adapter,
        status: status.status,
        currentStep: status.step,
        qualityScore: status.quality,
        iteration: status.iteration,
        maxIterations: status.maxIterations,
        error: status.error,
      });
    },
  });
}
```

## Usage

### Basic (Default)

```bash
hex feature dev webhook-notifications
```

Shows the clean progress view with keyboard controls.

### Verbose (Old Behavior)

```bash
hex feature dev webhook-notifications --verbose
```

Shows full agent logs (for debugging or CI/CD).

### From Claude Code Skill

```markdown
/hex-feature-dev
```

Prompts for feature name, then starts the workflow.

## Testing

### Unit Tests

```typescript
// tests/unit/feature-progress-orchestrator.test.ts

import { describe, it, expect, beforeEach } from 'bun:test';
import { FeatureProgressOrchestrator } from '../../src/core/usecases/feature-progress-orchestrator.js';
import { InMemoryFileSystem } from '../helpers/in-memory-fs.js';

describe('FeatureProgressOrchestrator', () => {
  let fs: InMemoryFileSystem;
  let orchestrator: FeatureProgressOrchestrator;

  beforeEach(() => {
    fs = new InMemoryFileSystem();
    orchestrator = new FeatureProgressOrchestrator(fs);
  });

  it('starts a feature session', async () => {
    const session = await orchestrator.startFeature('webhook-notifications');

    expect(session.featureName).toBe('webhook-notifications');
    expect(session.currentPhase).toBe('init');
    expect(session.phases.length).toBe(11);
  });

  it('transitions phases automatically', async () => {
    await orchestrator.startFeature('test-feature');

    await orchestrator.completePhase('init');

    const session = orchestrator.getCurrentSession();
    expect(session?.currentPhase).toBe('specs');
  });

  it('updates agent status and rebuilds progress', async () => {
    await orchestrator.startFeature('test-feature');

    await orchestrator.updateAgent({
      agentName: 'git-adapter',
      adapter: 'git-adapter',
      status: 'running',
      currentStep: 'test',
      iteration: 1,
      maxIterations: 3,
    });

    const progress = await orchestrator.getProgress();
    const agent = progress.agents.find((a) => a.agentName === 'git-adapter');

    expect(agent?.status).toBe('running');
    expect(agent?.currentStep).toBe('test');
  });

  it('auto-adds blockers on failure', async () => {
    await orchestrator.startFeature('test-feature');

    await orchestrator.updateAgent({
      agentName: 'cli-adapter',
      adapter: 'cli-adapter',
      status: 'failed',
      currentStep: 'compile',
      iteration: 1,
      maxIterations: 3,
      error: 'Compilation failed: missing type',
    });

    const progress = await orchestrator.getProgress();

    expect(progress.blockers.length).toBe(1);
    expect(progress.blockers[0].agentName).toBe('cli-adapter');
    expect(progress.blockers[0].type).toBe('compile_error');
  });

  it('generates final report', async () => {
    await orchestrator.startFeature('test-feature');

    const report = await orchestrator.endFeature('PASS', 'abc1234');

    expect(report.featureName).toBe('test-feature');
    expect(report.verdict).toBe('PASS');
    expect(report.integrationCommit).toBe('abc1234');
    expect(report.durationSeconds).toBeGreaterThan(0);
  });
});
```

### Integration Test

```typescript
// tests/integration/feature-workflow.test.ts

import { describe, it, expect } from 'bun:test';
import { spawn } from 'node:child_process';

describe('Feature Workflow', () => {
  it('runs end-to-end with clean output', async () => {
    const proc = spawn('hex', ['feature', 'dev', 'test-webhook', '--verbose'], {
      cwd: '/path/to/test-project',
    });

    let stdout = '';
    proc.stdout.on('data', (chunk) => { stdout += chunk.toString(); });

    await new Promise((resolve) => proc.on('close', resolve));

    // Should NOT contain raw tool calls
    expect(stdout).not.toContain('Tool: Read');
    expect(stdout).not.toContain('Tool: Bash');

    // Should contain progress milestones
    expect(stdout).toContain('Phase 1/11: INIT');
    expect(stdout).toContain('Phase 2/11: SPECS');
    expect(stdout).toContain('Overall: 100%');
  });
});
```

## Migration Checklist

- [ ] Add `IFeatureProgressPort` to `composition-root.ts`
- [ ] Update `AppContext` type definition
- [ ] Add `hex feature dev` CLI command
- [ ] Update `feature-developer.yml` agent workflow
- [ ] Implement agent log redirection
- [ ] Add unit tests for `FeatureProgressOrchestrator`
- [ ] Add integration test for full workflow
- [ ] Update CLAUDE.md with new usage instructions
- [ ] Document keyboard shortcuts in help text
- [ ] Test with real feature (e.g., webhook-notifications)

## Troubleshooting

### Progress not updating

**Symptom**: Display shows "0% complete" even though agents are running.

**Cause**: Agents are not calling `ctx.featureProgress.updateAgent()`.

**Fix**: Ensure all spawned agents have `onStatusChange` callback wired.

### Logs still flooding console

**Symptom**: Raw tool calls visible in terminal.

**Cause**: Agents not redirecting stdout to log files.

**Fix**: Check that `spawnBackgroundAgent()` redirects `onStdout`/`onStderr`.

### Blockers not showing

**Symptom**: Agent fails but blocker list is empty.

**Cause**: Agent status update missing `error` field.

**Fix**: Pass `error: "..."` in `updateAgent()` call when status = 'failed'.

### Display garbled in CI

**Symptom**: ANSI codes visible as raw text in CI logs.

**Cause**: CI environment is not a TTY.

**Fix**: Use `--verbose` flag for CI/CD pipelines.

## Next Steps

After integration:

1. **Test with example feature**: Run `hex feature dev webhook-notifications` in a real project
2. **Measure noise reduction**: Compare line counts before/after (target: <50 lines)
3. **Gather user feedback**: Does the progress view answer "where am I?" quickly?
4. **Add dashboard integration**: Push progress to hex-hub for browser view
5. **Document best practices**: Update agent-writing guide with status event patterns
