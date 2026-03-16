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
  IArchAnalysisPort,
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

/** Maximum arch-validation refinement passes to avoid infinite loops. */
const MAX_ARCH_REFINE_PASSES = 2;

export class CodeGenerator implements ICodeGenerationPort {
  constructor(
    private readonly llm: ILLMPort,
    private readonly ast: IASTPort,
    private readonly build: IBuildPort,
    private readonly fs: IFileSystemPort,
    private readonly archAnalyzer?: IArchAnalysisPort,
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

    // Phase 1: Validate generated code compiles
    const project = { name: 'hex', rootPath: '.', language: lang, adapters: [] };
    const buildResult = await this.build.compile(project);
    let result = codeUnit;
    if (!buildResult.success) {
      result = await this.refineFromBuildErrors(codeUnit, buildResult.errors);
    }

    // Phase 2: Architecture validation feedback loop
    if (this.archAnalyzer) {
      result = await this.refineFromArchAnalysis(result);
    }

    return result;
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
          '',
          '## Rules to enforce during refinement',
          '- Only import from ports, never from other adapters or domain/ directly.',
          '- All dependencies must be constructor-injected as port interfaces.',
          '- Adapters with connections (DB, HTTP) must have close()/dispose().',
          '- Validate external input at system boundaries (HTTP bodies, CLI args).',
          '- Re-export domain types through ports if adapters need them.',
          '- Remove dead exports (only export what ports define or composition-root uses).',
          '- Use .js extensions on all relative imports.',
          '',
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

  // ── Architecture validation feedback loop ─────────────────

  private async refineFromArchAnalysis(unit: CodeUnit): Promise<CodeUnit> {
    if (!this.archAnalyzer) return unit;

    let current = unit;

    for (let pass = 0; pass < MAX_ARCH_REFINE_PASSES; pass++) {
      const [violations, deadExports] = await Promise.all([
        this.archAnalyzer.validateHexBoundaries('.'),
        this.archAnalyzer.findDeadExports('.'),
      ]);

      // Filter to findings relevant to the generated file
      const fileViolations = violations.filter(
        (v) => v.from.includes(current.filePath) || v.to.includes(current.filePath),
      );
      const fileDeadExports = deadExports.filter(
        (d) => d.filePath.includes(current.filePath),
      );

      if (fileViolations.length === 0 && fileDeadExports.length === 0) {
        return current; // Clean — no arch issues
      }

      // Convert arch findings to LintError format for the refine pipeline
      const archErrors: LintError[] = [
        ...fileViolations.map((v) => ({
          filePath: v.from,
          line: 0,
          column: 0,
          severity: 'error' as const,
          message: `Hex boundary violation: ${v.from} → ${v.to} (${v.rule})`,
          rule: 'hex-boundary',
        })),
        ...fileDeadExports.map((d) => ({
          filePath: d.filePath,
          line: 0,
          column: 0,
          severity: 'warning' as const,
          message: `Dead export: ${d.exportName} (${d.kind}) is never imported by any other file`,
          rule: 'dead-export',
        })),
      ];

      current = await this.refineFromFeedback(current, archErrors);
    }

    return current;
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
      'You are generating code for a hexagonal architecture project (hex).',
      '',
      '## Import Rules (STRICT)',
      '- Only import from ports (../../core/ports/index.js), never from other adapters.',
      '- Use .js extensions on all relative imports (NodeNext resolution).',
      '- Domain types used by adapters MUST be re-exported through ports — never import domain/ directly from adapters.',
      '- Only export symbols that are defined in a port interface or needed by composition-root. No dead exports.',
      '',
      '## Dependency Injection Rules',
      '- ALL dependencies MUST be injected via constructor, typed as port interfaces (never concrete classes).',
      '- Primary adapters receive use case ports. Secondary adapters implement output ports.',
      '- The composition-root is the ONLY place that instantiates concrete adapters.',
      '',
      '## Lifecycle & Resource Management',
      '- Any adapter that opens connections (DB, HTTP, file handles) MUST expose a close()/dispose() method.',
      '- The composition-root or main entry point MUST call close() on shutdown (process signals, server stop).',
      '- Use try/finally or AbortSignal for cleanup in async code.',
      '',
      '## Input Validation & Error Handling',
      '- Validate ALL external input at system boundaries (HTTP request bodies, CLI args, env vars).',
      '- Return proper error responses (400 for bad input, 404 for not found) — never let malformed data reach domain logic.',
      '- Domain functions should throw typed errors; adapters catch and translate to protocol-specific responses.',
      '',
      '## Testing Requirements',
      '- Generate a companion test file for every module.',
      '- Use London-school (mock-first) unit tests: mock port interfaces, test behavior not implementation.',
      '- Test edge cases: empty input, special characters, missing fields, concurrent access.',
      '- Primary adapter tests should cover all routes/commands including error responses.',
      '',
      '## TypeScript Config',
      '- moduleResolution MUST be "nodenext" (not "bundler") to match .js import extensions.',
      '- strict: true, no implicit any.',
      '',
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
