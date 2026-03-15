import { describe, it, expect } from 'bun:test';
import type { ILLMPort, IASTPort, IFileSystemPort, ASTSummary, TokenBudget, Message } from '../../src/core/ports/index.js';
import type { ISwarmPort, SwarmTask } from '../../src/core/ports/swarm.js';
import { WorkplanExecutor } from '../../src/core/usecases/workplan-executor.js';

// ─── Mock Factories ─────────────────────────────────────

const planJson = JSON.stringify({
  title: 'Auth Plan',
  steps: [
    { id: 'step-1', description: 'Create port', adapter: 'ports', dependencies: [] },
    { id: 'step-2', description: 'Build adapter', adapter: 'secondary', dependencies: ['step-1'] },
  ],
});

const llmResponse = (content: string) => ({ content, tokenUsage: { input: 50, output: 30 }, model: 'mock' });

function mockLLM(response: string): ILLMPort {
  return { prompt: async () => llmResponse(response), streamPrompt: async function* () { yield response; } };
}

function capturingLLM(response: string): ILLMPort & { captured: Message[][] } {
  const spy: ILLMPort & { captured: Message[][] } = {
    captured: [],
    prompt: async (_b, msgs) => { spy.captured.push(msgs); return llmResponse(response); },
    streamPrompt: async function* () { yield response; },
  };
  return spy;
}

function mockAST(): IASTPort {
  return {
    extractSummary: async (filePath: string, level: ASTSummary['level']): Promise<ASTSummary> => ({
      filePath, language: 'typescript', level,
      exports: [], imports: [], dependencies: [], lineCount: 10, tokenEstimate: 50,
    }),
    diffStructural: () => ({ added: [], removed: [], modified: [] }),
  };
}

function mockFS(): IFileSystemPort {
  return { read: async () => '', write: async () => {}, exists: async () => true, glob: async () => [] };
}

const SS = { id: 's1', topology: 'hierarchical' as const, agentCount: 0, activeTaskCount: 0, completedTaskCount: 0 };

function mockSwarm(): ISwarmPort & { createdTasks: string[] } {
  let tid = 0;
  const s: ISwarmPort & { createdTasks: string[] } = {
    createdTasks: [],
    init: async () => ({ ...SS, status: 'running' }), status: async () => ({ ...SS, status: 'idle' }),
    shutdown: async () => {}, completeTask: async () => {}, listTasks: async () => [],
    createTask: async (t) => {
      s.createdTasks.push(t.title);
      return { id: `t-${++tid}`, title: t.title, agentRole: t.agentRole, status: 'pending' } as SwarmTask;
    },
    spawnAgent: async (name, role) => ({ id: 'a1', name, role, status: 'active' }),
    terminateAgent: async () => {}, listAgents: async () => [],
    memoryStore: async () => {}, memoryRetrieve: async () => null, memorySearch: async () => [],
  };
  return s;
}

// ─── Tests ──────────────────────────────────────────────

describe('WorkplanExecutor.createPlan', () => {
  it('calls llm.prompt with requirements', async () => {
    const llm = capturingLLM(planJson);
    const exec = new WorkplanExecutor(llm, mockAST(), mockFS(), mockSwarm());
    await exec.createPlan(['Add auth', 'Add tests'], 'typescript');
    const userMsg = llm.captured[0].find((m) => m.role === 'user')!;
    expect(userMsg.content).toContain('Add auth');
    expect(userMsg.content).toContain('Add tests');
  });

  it('returns a Workplan with steps', async () => {
    const exec = new WorkplanExecutor(mockLLM(planJson), mockAST(), mockFS(), mockSwarm());
    const plan = await exec.createPlan(['Add auth'], 'typescript');
    expect(plan.steps).toHaveLength(2);
    expect(plan.title).toBe('Auth Plan');
    expect(plan.steps[0].description).toBe('Create port');
  });
});

describe('WorkplanExecutor.executePlan', () => {
  const plan = { id: 'plan-1', title: 'Test', estimatedTokenBudget: 1000, steps: [
    { id: 's1', description: 'First', adapter: 'core', dependencies: [] },
    { id: 's2', description: 'Second', adapter: 'api', dependencies: ['s1'] },
  ] };

  it('yields StepResults in dependency order', async () => {
    const exec = new WorkplanExecutor(mockLLM('done'), mockAST(), mockFS(), mockSwarm());
    const ids: string[] = [];
    for await (const r of exec.executePlan(plan)) {
      if (r.status === 'passed') ids.push(r.stepId);
    }
    expect(ids).toEqual(['s1', 's2']);
  });

  it('registers tasks with swarm port', async () => {
    const swarm = mockSwarm();
    const exec = new WorkplanExecutor(mockLLM('done'), mockAST(), mockFS(), swarm);
    for await (const _r of exec.executePlan(plan)) { /* drain */ }
    expect(swarm.createdTasks).toContain('First');
    expect(swarm.createdTasks).toContain('Second');
  });

  it('handles step failures gracefully', async () => {
    const failSwarm = mockSwarm();
    failSwarm.createTask = async () => { throw new Error('Swarm down'); };
    const exec = new WorkplanExecutor(mockLLM('done'), mockAST(), mockFS(), failSwarm);
    const results = [];
    for await (const r of exec.executePlan(plan)) results.push(r);
    const failed = results.filter((r) => r.status === 'failed');
    expect(failed.length).toBeGreaterThan(0);
    expect(failed[0].errors![0]).toContain('Swarm down');
  });
});
