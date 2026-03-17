/**
 * Agent Executor Domain Types
 *
 * Value objects for the agentic execution model. These represent tasks,
 * results, tool calls, and comparison reports independent of whether
 * the executor is Claude Code CLI or a direct Anthropic API adapter.
 */

// ─── Tool Definitions ───────────────────────────────────

export type AgentToolName =
  | 'read_file'
  | 'write_file'
  | 'edit_file'
  | 'glob_files'
  | 'grep_search'
  | 'bash'
  | 'list_directory';

export interface AgentToolCall {
  id: string;
  name: AgentToolName | string;
  input: Record<string, unknown>;
}

export interface AgentToolResult {
  toolCallId: string;
  name: string;
  output: string;
  isError: boolean;
}

// ─── Context Loading ────────────────────────────────────

export interface AgentContextSource {
  type: 'claude-md' | 'skill' | 'agent-definition' | 'hook';
  name: string;
  content: string;
  origin: string; // file path or identifier
}

export interface AgentContext {
  systemPrompt: string;
  sources: AgentContextSource[];
  projectPath: string;
  /** Merged CLAUDE.md content (global + project) */
  claudeMd: string;
}

// ─── Task & Result ──────────────────────────────────────

export interface AgentTask {
  id: string;
  /** Natural language description of what to accomplish */
  prompt: string;
  /** Working directory for file operations */
  projectPath: string;
  /** Optional agent role (planner, coder, tester, etc.) */
  role?: string;
  /** Optional agent definition name to load */
  agentDefinition?: string;
  /** Optional skill names to inject */
  skills?: string[];
  /** Maximum conversation turns before aborting */
  maxTurns?: number;
  /** Maximum tokens to spend */
  maxTokens?: number;
  /** Model to use (adapter-specific default if omitted) */
  model?: string;
}

export interface AgentMetrics {
  totalInputTokens: number;
  totalOutputTokens: number;
  totalTurns: number;
  totalToolCalls: number;
  toolCallBreakdown: Record<string, number>;
  durationMs: number;
  model: string;
}

export type AgentResultStatus = 'success' | 'failure' | 'max_turns_reached' | 'error';

export interface AgentResult {
  taskId: string;
  status: AgentResultStatus;
  /** Final text output from the agent */
  output: string;
  /** Files created or modified */
  filesChanged: string[];
  /** All tool calls made during execution */
  toolCalls: AgentToolCall[];
  metrics: AgentMetrics;
  /** Error message if status is 'error' */
  error?: string;
}

// ─── Comparison ─────────────────────────────────────────

export type ExecutorBackend = 'claude-code' | 'anthropic-api';

export interface ComparisonEntry {
  backend: ExecutorBackend;
  task: AgentTask;
  result: AgentResult;
  /** Build result after agent finished */
  buildSuccess: boolean;
  /** Test pass rate (0-1) */
  testPassRate: number;
  /** Architecture health score (0-100) from hex analyze */
  archHealthScore: number;
}

export interface ComparisonReport {
  id: string;
  createdAt: string;
  /** The shared specification both swarms executed */
  specification: string;
  entries: [ComparisonEntry, ComparisonEntry];
  winner: ExecutorBackend | 'tie';
  summary: {
    tokenEfficiency: { claudeCode: number; anthropicApi: number };
    speed: { claudeCode: number; anthropicApi: number };
    quality: { claudeCode: number; anthropicApi: number };
  };
}
