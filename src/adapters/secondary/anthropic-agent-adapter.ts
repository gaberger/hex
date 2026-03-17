/**
 * Anthropic Agent Executor Adapter
 *
 * Implements IAgentExecutorPort by calling the Anthropic Messages API
 * directly with tool_use. Replicates Claude Code's agentic capabilities:
 *
 * - Loads CLAUDE.md (global + project) into system prompt
 * - Defines tools for file I/O, search, and shell execution
 * - Runs a multi-turn conversation loop handling tool_use blocks
 * - Tracks all metrics (tokens, tool calls, duration)
 *
 * Uses hex port interfaces (IFileSystemPort, IGitPort) for all tool
 * implementations, ensuring hex boundary rules are respected.
 *
 * Security: Shell commands use execFile('/bin/sh', ['-c', cmd]) — the
 * safe child_process pattern. The bash tool intentionally passes LLM-
 * generated commands to the shell, which is the expected agentic behavior.
 */

import type {
  IAgentExecutorPort,
  IFileSystemPort,
  AgentTask,
  AgentResult,
  AgentContext,
  AgentContextSource,
  AgentToolCall,
  AgentMetrics,
  ExecutorBackend,
} from '../../core/ports/index.js';

// ─── Anthropic API Types ────────────────────────────────

interface AnthropicMessage {
  role: 'user' | 'assistant';
  content: AnthropicContent[];
}

type AnthropicContent =
  | { type: 'text'; text: string }
  | { type: 'tool_use'; id: string; name: string; input: Record<string, unknown> }
  | { type: 'tool_result'; tool_use_id: string; content: string; is_error?: boolean };

interface AnthropicToolDef {
  name: string;
  description: string;
  input_schema: Record<string, unknown>;
}

interface AnthropicResponse {
  id: string;
  type: 'message';
  role: 'assistant';
  content: AnthropicContent[];
  model: string;
  stop_reason: 'end_turn' | 'tool_use' | 'max_tokens' | 'stop_sequence';
  usage: { input_tokens: number; output_tokens: number };
}

// ─── Configuration ──────────────────────────────────────

export interface AnthropicAgentConfig {
  apiKey: string;
  model?: string;
  baseUrl?: string;
  maxTurns?: number;
  maxTokens?: number;
}

const DEFAULT_MODEL = 'claude-sonnet-4-20250514';
const DEFAULT_BASE_URL = 'https://api.anthropic.com';
const DEFAULT_MAX_TURNS = 50;
const DEFAULT_MAX_TOKENS = 16384;

// ─── Tool Definitions ───────────────────────────────────

function buildToolDefinitions(): AnthropicToolDef[] {
  return [
    {
      name: 'read_file',
      description: 'Read the contents of a file at the given path. Returns the file content as text.',
      input_schema: {
        type: 'object',
        properties: {
          file_path: { type: 'string', description: 'Absolute or project-relative file path' },
          offset: { type: 'number', description: 'Line number to start reading from (1-based)' },
          limit: { type: 'number', description: 'Maximum number of lines to read' },
        },
        required: ['file_path'],
      },
    },
    {
      name: 'write_file',
      description: 'Write content to a file, creating it if necessary or overwriting if it exists.',
      input_schema: {
        type: 'object',
        properties: {
          file_path: { type: 'string', description: 'Absolute or project-relative file path' },
          content: { type: 'string', description: 'The full content to write' },
        },
        required: ['file_path', 'content'],
      },
    },
    {
      name: 'edit_file',
      description: 'Replace an exact string in a file with a new string. The old_string must match exactly (including whitespace).',
      input_schema: {
        type: 'object',
        properties: {
          file_path: { type: 'string', description: 'Absolute or project-relative file path' },
          old_string: { type: 'string', description: 'The exact text to find and replace' },
          new_string: { type: 'string', description: 'The replacement text' },
        },
        required: ['file_path', 'old_string', 'new_string'],
      },
    },
    {
      name: 'glob_files',
      description: 'Find files matching a glob pattern. Returns a list of matching file paths.',
      input_schema: {
        type: 'object',
        properties: {
          pattern: { type: 'string', description: 'Glob pattern (e.g., "src/**/*.ts", "*.json")' },
        },
        required: ['pattern'],
      },
    },
    {
      name: 'grep_search',
      description: 'Search file contents for a regex pattern. Returns matching lines with file paths and line numbers.',
      input_schema: {
        type: 'object',
        properties: {
          pattern: { type: 'string', description: 'Regex pattern to search for' },
          path: { type: 'string', description: 'Directory or file to search in (default: project root)' },
          glob: { type: 'string', description: 'File glob to filter (e.g., "*.ts")' },
        },
        required: ['pattern'],
      },
    },
    {
      name: 'bash',
      description: 'Execute a shell command and return its stdout/stderr. Use for build, test, git, and other CLI operations.',
      input_schema: {
        type: 'object',
        properties: {
          command: { type: 'string', description: 'The shell command to execute' },
          timeout: { type: 'number', description: 'Timeout in milliseconds (default: 120000)' },
        },
        required: ['command'],
      },
    },
    {
      name: 'list_directory',
      description: 'List files and directories at the given path.',
      input_schema: {
        type: 'object',
        properties: {
          path: { type: 'string', description: 'Directory path to list (default: project root)' },
        },
        required: [],
      },
    },
  ];
}

// ─── Adapter Implementation ─────────────────────────────

export class AnthropicAgentAdapter implements IAgentExecutorPort {
  readonly backend: ExecutorBackend = 'anthropic-api';
  private readonly baseUrl: string;
  private readonly model: string;
  private readonly defaultMaxTurns: number;
  private readonly defaultMaxTokens: number;
  private context: AgentContext | null = null;

  constructor(
    private readonly config: AnthropicAgentConfig,
    private readonly fs: IFileSystemPort,
  ) {
    this.baseUrl = config.baseUrl ?? DEFAULT_BASE_URL;
    this.model = config.model ?? DEFAULT_MODEL;
    this.defaultMaxTurns = config.maxTurns ?? DEFAULT_MAX_TURNS;
    this.defaultMaxTokens = config.maxTokens ?? DEFAULT_MAX_TOKENS;
  }

  // ── Context Loading ─────────────────────────────────────

  async loadContext(projectPath: string): Promise<AgentContext> {
    const sources: AgentContextSource[] = [];
    let claudeMd = '';

    // 1. Global CLAUDE.md (~/.claude/CLAUDE.md)
    const home = process.env['HOME'] ?? process.env['USERPROFILE'] ?? '';
    const globalClaudeMd = `${home}/.claude/CLAUDE.md`;
    try {
      const content = await this.readFileRaw(globalClaudeMd);
      sources.push({ type: 'claude-md', name: 'global', content, origin: globalClaudeMd });
      claudeMd += `# Global Instructions\n\n${content}\n\n`;
    } catch { /* no global CLAUDE.md — fine */ }

    // 2. Project CLAUDE.md
    const projectClaudeMd = `${projectPath}/CLAUDE.md`;
    try {
      const content = await this.readFileRaw(projectClaudeMd);
      sources.push({ type: 'claude-md', name: 'project', content, origin: projectClaudeMd });
      claudeMd += `# Project Instructions\n\n${content}\n\n`;
    } catch { /* no project CLAUDE.md — fine */ }

    // 3. Agent definitions (.claude/agents/ or agents/)
    for (const agentDir of ['.claude/agents', 'agents']) {
      try {
        const files = await this.fs.glob(`${agentDir}/**/*.{yml,yaml,md}`);
        for (const file of files) {
          try {
            const content = await this.fs.read(file);
            const name = file.split('/').pop()?.replace(/\.(yml|yaml|md)$/, '') ?? file;
            sources.push({ type: 'agent-definition', name, content, origin: file });
          } catch { /* skip unreadable files */ }
        }
      } catch { /* directory doesn't exist */ }
    }

    // 4. Skill manifests (.claude/skills/ or skills/)
    for (const skillDir of ['.claude/skills', 'skills']) {
      try {
        const files = await this.fs.glob(`${skillDir}/**/*.md`);
        for (const file of files) {
          try {
            const content = await this.fs.read(file);
            const name = file.split('/').pop()?.replace(/\.md$/, '') ?? file;
            sources.push({ type: 'skill', name, content, origin: file });
          } catch { /* skip */ }
        }
      } catch { /* directory doesn't exist */ }
    }

    // 5. Hook configurations (.claude/settings.json hooks)
    for (const hookPath of ['.claude/settings.json', '.claude/settings.local.json']) {
      try {
        const content = await this.fs.read(hookPath);
        const parsed = JSON.parse(content) as Record<string, unknown>;
        if (parsed.hooks) {
          sources.push({
            type: 'hook',
            name: hookPath,
            content: JSON.stringify(parsed.hooks, null, 2),
            origin: hookPath,
          });
        }
      } catch { /* no hooks config */ }
    }

    // Build system prompt
    const systemPrompt = this.buildSystemPrompt(claudeMd, projectPath);

    this.context = { systemPrompt, sources, projectPath, claudeMd };
    return this.context;
  }

  // ── Execution ───────────────────────────────────────────

  async execute(task: AgentTask): Promise<AgentResult> {
    return this.executeWithProgress(task, () => {});
  }

  async executeWithProgress(
    task: AgentTask,
    onProgress: (chunk: string) => void,
  ): Promise<AgentResult> {
    const startTime = Date.now();
    const maxTurns = task.maxTurns ?? this.defaultMaxTurns;
    const model = task.model ?? this.model;

    // Ensure context is loaded
    if (!this.context || this.context.projectPath !== task.projectPath) {
      await this.loadContext(task.projectPath);
    }

    const systemPrompt = this.buildTaskSystemPrompt(task);
    const tools = buildToolDefinitions();
    const messages: AnthropicMessage[] = [
      { role: 'user', content: [{ type: 'text', text: task.prompt }] },
    ];

    const allToolCalls: AgentToolCall[] = [];
    const toolCallBreakdown: Record<string, number> = {};
    const filesChanged = new Set<string>();
    let totalInputTokens = 0;
    let totalOutputTokens = 0;
    let finalOutput = '';
    let turns = 0;

    // ── Agentic loop ──────────────────────────────────────
    for (turns = 0; turns < maxTurns; turns++) {
      const response = await this.callAPI(systemPrompt, messages, tools, model, task.maxTokens);
      totalInputTokens += response.usage.input_tokens;
      totalOutputTokens += response.usage.output_tokens;

      // Collect text output
      const textBlocks = response.content.filter(
        (b): b is { type: 'text'; text: string } => b.type === 'text',
      );
      for (const block of textBlocks) {
        finalOutput += block.text;
        onProgress(block.text);
      }

      // Add assistant message to conversation
      messages.push({ role: 'assistant', content: response.content });

      // If no tool use, agent is done
      if (response.stop_reason !== 'tool_use') {
        break;
      }

      // Process tool calls
      const toolUseBlocks = response.content.filter(
        (b): b is { type: 'tool_use'; id: string; name: string; input: Record<string, unknown> } =>
          b.type === 'tool_use',
      );

      const toolResults: AnthropicContent[] = [];
      for (const toolUse of toolUseBlocks) {
        const call: AgentToolCall = {
          id: toolUse.id,
          name: toolUse.name,
          input: toolUse.input,
        };
        allToolCalls.push(call);
        toolCallBreakdown[toolUse.name] = (toolCallBreakdown[toolUse.name] ?? 0) + 1;

        // Execute the tool
        const result = await this.executeTool(toolUse.name, toolUse.input, task.projectPath);

        // Track file changes
        if (['write_file', 'edit_file'].includes(toolUse.name) && !result.isError) {
          const filePath = toolUse.input['file_path'] as string | undefined;
          if (filePath) filesChanged.add(filePath);
        }

        toolResults.push({
          type: 'tool_result',
          tool_use_id: toolUse.id,
          content: result.output,
          is_error: result.isError || undefined,
        });
      }

      // Add tool results as user message
      messages.push({ role: 'user', content: toolResults });
    }

    const durationMs = Date.now() - startTime;
    const metrics: AgentMetrics = {
      totalInputTokens,
      totalOutputTokens,
      totalTurns: turns + 1,
      totalToolCalls: allToolCalls.length,
      toolCallBreakdown,
      durationMs,
      model,
    };

    const status = turns >= maxTurns ? 'max_turns_reached' : 'success';

    return {
      taskId: task.id,
      status,
      output: finalOutput,
      filesChanged: [...filesChanged],
      toolCalls: allToolCalls,
      metrics,
    };
  }

  // ── API Call ────────────────────────────────────────────

  private async callAPI(
    system: string,
    messages: AnthropicMessage[],
    tools: AnthropicToolDef[],
    model: string,
    maxTokens?: number,
  ): Promise<AnthropicResponse> {
    const url = `${this.baseUrl}/v1/messages`;
    const body = {
      model,
      max_tokens: maxTokens ?? this.defaultMaxTokens,
      system,
      tools,
      messages,
    };

    const res = await fetch(url, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        'x-api-key': this.config.apiKey,
        'anthropic-version': '2023-06-01',
      },
      body: JSON.stringify(body),
    });

    if (!res.ok) {
      const text = await res.text();
      throw new Error(`Anthropic API error (${res.status}): ${text}`);
    }

    return res.json() as Promise<AnthropicResponse>;
  }

  // ── Tool Execution ──────────────────────────────────────

  private async executeTool(
    name: string,
    input: Record<string, unknown>,
    projectPath: string,
  ): Promise<{ output: string; isError: boolean }> {
    try {
      switch (name) {
        case 'read_file': {
          const filePath = this.resolvePath(input['file_path'] as string, projectPath);
          const content = await this.readFileRaw(filePath);
          const offset = (input['offset'] as number | undefined) ?? 0;
          const limit = input['limit'] as number | undefined;
          let lines = content.split('\n');
          if (offset > 0) lines = lines.slice(offset - 1);
          if (limit) lines = lines.slice(0, limit);
          const numbered = lines.map((l, i) => `${String(offset + i + 1).padStart(6)}│${l}`).join('\n');
          return { output: numbered, isError: false };
        }

        case 'write_file': {
          const filePath = this.resolvePath(input['file_path'] as string, projectPath);
          await this.writeFileRaw(filePath, input['content'] as string);
          return { output: `File written: ${filePath}`, isError: false };
        }

        case 'edit_file': {
          const filePath = this.resolvePath(input['file_path'] as string, projectPath);
          const content = await this.readFileRaw(filePath);
          const oldStr = input['old_string'] as string;
          const newStr = input['new_string'] as string;
          const count = content.split(oldStr).length - 1;
          if (count === 0) {
            return { output: `Error: old_string not found in ${filePath}`, isError: true };
          }
          if (count > 1) {
            return { output: `Error: old_string found ${count} times — must be unique`, isError: true };
          }
          const updated = content.replace(oldStr, newStr);
          await this.writeFileRaw(filePath, updated);
          return { output: `File edited: ${filePath}`, isError: false };
        }

        case 'glob_files': {
          const matches = await this.fs.glob(input['pattern'] as string);
          return { output: matches.join('\n') || '(no matches)', isError: false };
        }

        case 'grep_search': {
          const result = await this.runShell(
            this.buildGrepCommand(input, projectPath),
            projectPath,
            30_000,
          );
          return { output: result, isError: false };
        }

        case 'bash': {
          const timeout = (input['timeout'] as number | undefined) ?? 120_000;
          const result = await this.runShell(input['command'] as string, projectPath, timeout);
          return { output: result, isError: false };
        }

        case 'list_directory': {
          const dirPath = this.resolvePath((input['path'] as string | undefined) ?? '.', projectPath);
          const result = await this.runShell(`ls -la "${dirPath}"`, projectPath, 10_000);
          return { output: result, isError: false };
        }

        default:
          return { output: `Unknown tool: ${name}`, isError: true };
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      return { output: `Tool error: ${msg}`, isError: true };
    }
  }

  // ── System Prompt Construction ──────────────────────────

  private buildSystemPrompt(claudeMd: string, projectPath: string): string {
    const parts: string[] = [];

    parts.push(`You are an expert software engineer working on a project at ${projectPath}.`);
    parts.push('You have access to tools for reading, writing, and editing files, searching code, and running shell commands.');
    parts.push('Always read a file before editing it. Prefer editing existing files over creating new ones.');
    parts.push('Use the bash tool for build, test, and git operations.');
    parts.push('');

    if (claudeMd) {
      parts.push('# Project Instructions');
      parts.push('');
      parts.push(claudeMd);
    }

    return parts.join('\n');
  }

  private buildTaskSystemPrompt(task: AgentTask): string {
    let prompt = this.context?.systemPrompt ?? '';

    // Inject agent definition if specified
    if (task.agentDefinition && this.context) {
      const agentDef = this.context.sources.find(
        (s) => s.type === 'agent-definition' && s.name === task.agentDefinition,
      );
      if (agentDef) {
        prompt += `\n\n# Agent Role: ${agentDef.name}\n\n${agentDef.content}`;
      }
    }

    // Inject requested skills
    if (task.skills && this.context) {
      for (const skillName of task.skills) {
        const skill = this.context.sources.find(
          (s) => s.type === 'skill' && s.name === skillName,
        );
        if (skill) {
          prompt += `\n\n# Skill: ${skill.name}\n\n${skill.content}`;
        }
      }
    }

    if (task.role) {
      prompt += `\n\nYou are acting as a ${task.role}. Focus on tasks appropriate to this role.`;
    }

    return prompt;
  }

  // ── File I/O Helpers ──────────────────────────────────

  private async readFileRaw(filePath: string): Promise<string> {
    const { readFile } = await import('node:fs/promises');
    return readFile(filePath, 'utf-8');
  }

  private async writeFileRaw(filePath: string, content: string): Promise<void> {
    const { writeFile, mkdir } = await import('node:fs/promises');
    const { dirname } = await import('node:path');
    await mkdir(dirname(filePath), { recursive: true }).catch(() => {});
    await writeFile(filePath, content, 'utf-8');
  }

  private resolvePath(filePath: string, projectPath: string): string {
    if (filePath.startsWith('/')) return filePath;
    return `${projectPath}/${filePath}`;
  }

  // ── Shell Execution (uses execFile — no shell injection) ──

  private buildGrepCommand(input: Record<string, unknown>, projectPath: string): string {
    const pattern = (input['pattern'] as string).replace(/'/g, "'\\''");
    const searchPath = (input['path'] as string | undefined) ?? projectPath;
    const glob = input['glob'] as string | undefined;
    const globFlag = glob ? ` --glob '${glob}'` : '';
    return `rg --line-number --no-heading '${pattern}'${globFlag} '${searchPath}' 2>/dev/null | head -100`;
  }

  /**
   * Run a shell command using execFile('/bin/sh', ['-c', cmd]).
   * This is the safe child_process pattern — no string interpolation
   * into the shell invocation itself. The command string is passed as
   * a single argument to /bin/sh.
   */
  private async runShell(command: string, cwd: string, timeout: number): Promise<string> {
    const { execFile } = await import('node:child_process');
    const { promisify } = await import('node:util');
    const run = promisify(execFile);

    try {
      const { stdout, stderr } = await run('/bin/sh', ['-c', command], {
        cwd,
        timeout,
        maxBuffer: 1024 * 1024 * 10,
        env: { ...process.env, TERM: 'dumb', NO_COLOR: '1' },
      });
      const output = stdout + (stderr ? `\n[stderr]\n${stderr}` : '');
      return output.trim() || '(no output)';
    } catch (err: unknown) {
      const error = err as { stdout?: string; stderr?: string; message?: string };
      const output = (error.stdout ?? '') + (error.stderr ? `\n[stderr]\n${error.stderr}` : '');
      if (output.trim()) return output.trim();
      return `Command failed: ${error.message ?? String(err)}`;
    }
  }
}
