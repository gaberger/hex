/**
 * Claude Code Executor Adapter
 *
 * Implements IAgentExecutorPort by delegating to the Claude Code CLI
 * (`claude` command). This is the baseline for comparison — it uses
 * the same tool that Claude Code's Agent tool uses internally.
 *
 * Invokes `claude -p --output-format json` for non-interactive execution.
 */

import type {
  IAgentExecutorPort,
  IFileSystemPort,
  AgentTask,
  AgentResult,
  AgentContext,
  AgentContextSource,
  AgentMetrics,
  ExecutorBackend,
} from '../../core/ports/index.js';

export interface ClaudeCodeExecutorConfig {
  /** Path to claude CLI binary (default: 'claude') */
  binaryPath?: string;
  /** Additional CLI flags */
  extraFlags?: string[];
  /** Model override */
  model?: string;
  /** Max tokens for response */
  maxTokens?: number;
}

export class ClaudeCodeExecutorAdapter implements IAgentExecutorPort {
  readonly backend: ExecutorBackend = 'claude-code';
  private context: AgentContext | null = null;
  private readonly binaryPath: string;
  private readonly extraFlags: string[];

  constructor(
    private readonly config: ClaudeCodeExecutorConfig,
    private readonly fs: IFileSystemPort,
  ) {
    this.binaryPath = config.binaryPath ?? 'claude';
    this.extraFlags = config.extraFlags ?? [];
  }

  async loadContext(projectPath: string): Promise<AgentContext> {
    const sources: AgentContextSource[] = [];
    let claudeMd = '';

    // Claude Code loads CLAUDE.md automatically, but we track it for reporting
    const home = process.env['HOME'] ?? process.env['USERPROFILE'] ?? '';
    for (const [name, path] of [
      ['global', `${home}/.claude/CLAUDE.md`],
      ['project', `${projectPath}/CLAUDE.md`],
    ] as const) {
      try {
        const content = await this.readFileRaw(path);
        sources.push({ type: 'claude-md', name, content, origin: path });
        claudeMd += content + '\n\n';
      } catch { /* not present */ }
    }

    // Track agent definitions
    for (const dir of ['.claude/agents', 'agents']) {
      try {
        const files = await this.fs.glob(`${dir}/**/*.{yml,yaml,md}`);
        for (const file of files) {
          try {
            const content = await this.fs.read(file);
            const name = file.split('/').pop()?.replace(/\.(yml|yaml|md)$/, '') ?? file;
            sources.push({ type: 'agent-definition', name, content, origin: file });
          } catch { /* skip */ }
        }
      } catch { /* no dir */ }
    }

    this.context = {
      systemPrompt: '(managed by Claude Code CLI)',
      sources,
      projectPath,
      claudeMd,
    };
    return this.context;
  }

  async execute(task: AgentTask): Promise<AgentResult> {
    return this.executeWithProgress(task, () => {});
  }

  async executeWithProgress(
    task: AgentTask,
    onProgress: (chunk: string) => void,
  ): Promise<AgentResult> {
    const startTime = Date.now();

    if (!this.context || this.context.projectPath !== task.projectPath) {
      await this.loadContext(task.projectPath);
    }

    const args = this.buildArgs(task);

    try {
      const output = await this.runClaude(args, task.projectPath, onProgress);
      const durationMs = Date.now() - startTime;

      // Parse JSON output if available
      const parsed = this.parseOutput(output);

      const metrics: AgentMetrics = {
        totalInputTokens: parsed.inputTokens,
        totalOutputTokens: parsed.outputTokens,
        totalTurns: parsed.turns,
        totalToolCalls: parsed.toolCalls,
        toolCallBreakdown: parsed.toolCallBreakdown,
        durationMs,
        model: parsed.model ?? this.config.model ?? 'unknown',
      };

      return {
        taskId: task.id,
        status: 'success',
        output: parsed.text,
        filesChanged: parsed.filesChanged,
        toolCalls: [],
        metrics,
      };
    } catch (err) {
      const durationMs = Date.now() - startTime;
      const msg = err instanceof Error ? err.message : String(err);
      return {
        taskId: task.id,
        status: 'error',
        output: '',
        filesChanged: [],
        toolCalls: [],
        metrics: {
          totalInputTokens: 0,
          totalOutputTokens: 0,
          totalTurns: 0,
          totalToolCalls: 0,
          toolCallBreakdown: {},
          durationMs,
          model: this.config.model ?? 'unknown',
        },
        error: msg,
      };
    }
  }

  // ── CLI Invocation ──────────────────────────────────────

  private buildArgs(task: AgentTask): string[] {
    const args: string[] = [
      '-p',                           // non-interactive (print mode)
      '--output-format', 'json',      // structured output
    ];

    if (this.config.model) {
      args.push('--model', this.config.model);
    }
    if (task.maxTokens) {
      args.push('--max-turns', String(task.maxTurns ?? 50));
    }

    args.push(...this.extraFlags);

    // The prompt goes last
    args.push(task.prompt);

    return args;
  }

  private async runClaude(
    args: string[],
    cwd: string,
    onProgress: (chunk: string) => void,
  ): Promise<string> {
    const { spawn } = await import('node:child_process');

    return new Promise<string>((resolve, reject) => {
      const child = spawn(this.binaryPath, args, {
        cwd,
        env: { ...process.env, TERM: 'dumb', NO_COLOR: '1' },
        stdio: ['ignore', 'pipe', 'pipe'],
      });

      let stdout = '';
      let stderr = '';

      child.stdout.on('data', (data: Buffer) => {
        const chunk = data.toString();
        stdout += chunk;
        onProgress(chunk);
      });

      child.stderr.on('data', (data: Buffer) => {
        stderr += data.toString();
      });

      child.on('close', (code) => {
        if (code === 0) {
          resolve(stdout);
        } else {
          reject(new Error(`claude exited with code ${code}: ${stderr || stdout}`));
        }
      });

      child.on('error', (err) => {
        reject(new Error(`Failed to spawn claude: ${err.message}`));
      });
    });
  }

  // ── Output Parsing ────────────────────────────────────

  private parseOutput(raw: string): {
    text: string;
    inputTokens: number;
    outputTokens: number;
    turns: number;
    toolCalls: number;
    toolCallBreakdown: Record<string, number>;
    filesChanged: string[];
    model: string | null;
  } {
    try {
      // Claude Code JSON output format
      const parsed = JSON.parse(raw) as Record<string, unknown>;
      const result = parsed['result'] as string | undefined;
      const usage = parsed['usage'] as Record<string, number> | undefined;
      const model = parsed['model'] as string | undefined;

      return {
        text: result ?? raw,
        inputTokens: usage?.['input_tokens'] ?? 0,
        outputTokens: usage?.['output_tokens'] ?? 0,
        turns: (parsed['num_turns'] as number | undefined) ?? 1,
        toolCalls: 0,
        toolCallBreakdown: {},
        filesChanged: [],
        model: model ?? null,
      };
    } catch {
      // Not JSON — plain text output
      return {
        text: raw,
        inputTokens: 0,
        outputTokens: 0,
        turns: 1,
        toolCalls: 0,
        toolCallBreakdown: {},
        filesChanged: [],
        model: null,
      };
    }
  }

  private async readFileRaw(filePath: string): Promise<string> {
    const { readFile } = await import('node:fs/promises');
    return readFile(filePath, 'utf-8');
  }
}
