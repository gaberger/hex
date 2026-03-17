/**
 * Agent Executor Port
 *
 * Abstracts the execution of agentic coding tasks. Both Claude Code CLI
 * and the direct Anthropic API adapter implement this port, enabling
 * head-to-head comparison of execution strategies.
 *
 * This port sits at the secondary (driven) boundary — use cases call
 * it to delegate autonomous coding work to an LLM-backed agent.
 */

import type {
  AgentTask,
  AgentResult,
  AgentContext,
  ExecutorBackend,
  ComparisonReport,
} from '../domain/agent-executor-types.js';

export interface IAgentExecutorPort {
  /** Which backend this executor uses */
  readonly backend: ExecutorBackend;

  /**
   * Load context for a project — reads CLAUDE.md files, agent definitions,
   * skill manifests, and hook configurations. The returned AgentContext
   * is passed implicitly to execute() calls.
   */
  loadContext(projectPath: string): Promise<AgentContext>;

  /**
   * Execute an agentic task to completion. The executor runs a multi-turn
   * conversation loop with tool use until the task is done, max turns
   * are reached, or an error occurs.
   */
  execute(task: AgentTask): Promise<AgentResult>;

  /**
   * Execute with a progress callback for streaming status updates.
   * The callback receives partial output text as it arrives.
   */
  executeWithProgress(
    task: AgentTask,
    onProgress: (chunk: string) => void,
  ): Promise<AgentResult>;
}

/** Input port for comparing two agent executors head-to-head. */
export interface IComparisonPort {
  compare(
    specification: string,
    taskTemplate: Omit<AgentTask, 'id' | 'projectPath'>,
    onProgress?: (backend: ExecutorBackend, chunk: string) => void,
  ): Promise<ComparisonReport>;
}
