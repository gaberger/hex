/**
 * Code Generator use case -- implements ICodeGenerationPort.
 *
 * Orchestrates LLM calls to generate and refine code from specifications,
 * respecting hexagonal architecture boundaries.
 */
import type {
  ASTSummary,
  CodeUnit,
  IASTPort,
  IBuildPort,
  ICodeGenerationPort,
  IFileSystemPort,
  ILLMPort,
  Language,
  LintError,
  Message,
  Specification,
  TokenBudget,
} from '../ports/index.js';

const DEFAULT_BUDGET: TokenBudget = {
  maxTokens: 16000,
  reservedForResponse: 4096,
  available: 11904,
};

export class CodeGenerator implements ICodeGenerationPort {
  constructor(
    private readonly llm: ILLMPort,
    private readonly ast: IASTPort,
    private readonly build: IBuildPort,
    private readonly fs: IFileSystemPort,
  ) {}

  async generateFromSpec(spec: Specification, lang: Language): Promise<CodeUnit> {
    const portSummaries = await this.loadPortSummaries();
    const systemPrompt = this.buildSystemPrompt(spec, portSummaries);

    const userContent = [
      `Generate a ${lang} implementation for: ${spec.title}`,
      '',
      '## Requirements',
      ...spec.requirements.map((r) => `- ${r}`),
      '',
      '## Constraints',
      ...spec.constraints.map((c) => `- ${c}`),
      '',
      spec.targetAdapter
        ? `Target adapter: ${spec.targetAdapter}`
        : 'Target: new module',
      '',
      'Respond with ONLY the complete file content. No markdown fences.',
    ].join('\n');

    const messages: Message[] = [
      { role: 'system', content: systemPrompt },
      { role: 'user', content: userContent },
    ];

    const response = await this.llm.prompt(DEFAULT_BUDGET, messages);
    const content = stripCodeFences(response.content);
    const filePath = spec.targetAdapter
      ? `src/adapters/secondary/${spec.targetAdapter}.ts`
      : `src/core/usecases/${toFileName(spec.title)}.ts`;

    await this.fs.write(filePath, content);
    const astSummary = await this.ast.extractSummary(filePath, 'L1');
    const codeUnit: CodeUnit = { filePath, language: lang, content, astSummary };

    // Validate generated code compiles before returning
    const project = { name: 'hex-intf', rootPath: '.', language: lang, adapters: [] };
    const buildResult = await this.build.compile(project);
    if (!buildResult.success) {
      const refined = await this.refineFromBuildErrors(codeUnit, buildResult.errors);
      return refined;
    }

    return codeUnit;
  }

  async refineFromFeedback(unit: CodeUnit, errors: LintError[]): Promise<CodeUnit> {
    const errorSummary = errors
      .map((e) => `${e.filePath}:${e.line}:${e.column} [${e.severity}] ${e.message} (${e.rule})`)
      .join('\n');

    const messages: Message[] = [
      {
        role: 'system',
        content: [
          'You are refining code for a hexagonal architecture project.',
          'Fix ALL reported errors while preserving the existing architecture.',
          'Only import from ports, never from other adapters.',
          'Respond with ONLY the complete corrected file content. No markdown fences.',
        ].join('\n'),
      },
      {
        role: 'user',
        content: [
          '## Current Code',
          unit.content,
          '',
          '## Errors to Fix',
          errorSummary,
          '',
          'Return the corrected file content.',
        ].join('\n'),
      },
    ];

    const response = await this.llm.prompt(DEFAULT_BUDGET, messages);
    const content = stripCodeFences(response.content);

    await this.fs.write(unit.filePath, content);
    const astSummary = await this.ast.extractSummary(unit.filePath, 'L1');

    return { ...unit, content, astSummary };
  }

  // ── Private helpers ───────────────────────────────────────

  private async refineFromBuildErrors(unit: CodeUnit, errors: string[]): Promise<CodeUnit> {
    const lintErrors: LintError[] = errors.map((msg) => ({
      filePath: unit.filePath,
      line: 0,
      column: 0,
      severity: 'error' as const,
      message: msg,
      rule: 'compile',
    }));
    return this.refineFromFeedback(unit, lintErrors);
  }

  private buildSystemPrompt(spec: Specification, portSummaries: ASTSummary[]): string {
    const portList = portSummaries
      .flatMap((s) => s.exports.map((e) => `  - ${e.kind} ${e.name}${e.signature ? `: ${e.signature}` : ''}`))
      .join('\n');

    return [
      'You are generating code for a hexagonal architecture project (hex-intf).',
      '',
      '## Rules',
      '- Only import from ports (../../core/ports/index.js), never from other adapters.',
      '- Use .js extensions on all relative imports (NodeNext resolution).',
      '- Implement the port interface exactly as defined.',
      '- Use constructor injection for dependencies.',
      '- No external SDK dependencies unless specified.',
      spec.targetAdapter
        ? `- This adapter implements a port interface for: ${spec.targetAdapter}`
        : '- This is a use case in the domain core.',
      '',
      '## Available Port Interfaces',
      portList,
      '',
      '## Target Language',
      spec.targetLanguage,
    ].join('\n');
  }

  private async loadPortSummaries(): Promise<ASTSummary[]> {
    const portFiles = await this.fs.glob('src/core/ports/**/*.ts');
    const summaries: ASTSummary[] = [];
    for (const file of portFiles) {
      try {
        const summary = await this.ast.extractSummary(file, 'L1');
        summaries.push(summary);
      } catch {
        // Skip files that cannot be parsed
      }
    }
    return summaries;
  }
}

// ── Module-level helpers ──────────────────────────────────────

function stripCodeFences(text: string): string {
  return text.replace(/^```\w*\n?/gm, '').replace(/```\s*$/gm, '').trim();
}

function toFileName(title: string): string {
  return title
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '');
}
