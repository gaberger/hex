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
    const [portSummaries, adjacentContext] = await Promise.all([
      this.loadPortSummaries(),
      this.loadAdjacentContext(spec),
    ]);
    const systemPrompt = this.buildSystemPrompt(spec, portSummaries, adjacentContext);

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
    const ext = langExtension(lang);
    const filePath = spec.targetAdapter
      ? `src/adapters/secondary/${spec.targetAdapter}${ext}`
      : `src/core/usecases/${toFileName(spec.title)}${ext}`;

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

  private buildSystemPrompt(spec: Specification, portSummaries: ASTSummary[], adjacentContext: string): string {
    const portList = portSummaries
      .flatMap((s) => s.exports.map((e) => `  - ${e.kind} ${e.name}${e.signature ? `: ${e.signature}` : ''}`))
      .join('\n');

    const langRules = buildLanguageRules(spec.targetLanguage as Language);

    return [
      'You are generating code for a hexagonal architecture project (hex).',
      '',
      '## Hexagonal Architecture Rules (ALL LANGUAGES)',
      '- Only import from ports, never from other adapters.',
      '- Domain types used by adapters MUST be accessed through ports — never import domain/ directly.',
      '- Only export symbols that are defined in a port interface or needed by composition-root.',
      '',
      '## Dependency Injection Rules',
      '- ALL dependencies MUST be injected via constructor (or function parameters in Go/Rust).',
      '- Primary adapters receive use case ports. Secondary adapters implement output ports.',
      '- The composition-root is the ONLY place that instantiates concrete adapters.',
      '',
      '## Lifecycle & Resource Management',
      '- Any adapter that opens connections (DB, HTTP, file handles) MUST expose a Close()/drop() method.',
      '- The composition-root or main entry point MUST call cleanup on shutdown.',
      '',
      '## Input Validation & Error Handling',
      '- Validate ALL external input at system boundaries (HTTP request bodies, CLI args, env vars).',
      '- Return proper error responses — never let malformed data reach domain logic.',
      '',
      '## Testing Requirements',
      '- Generate a companion test file for every module.',
      '- Mock port interfaces, test behavior not implementation.',
      '',
      langRules,
      '',
      spec.targetAdapter
        ? `- This adapter implements a port interface for: ${spec.targetAdapter}`
        : '- This is a use case in the domain core.',
      '',
      '## Available Port Interfaces',
      portList,
      '',
      adjacentContext ? `## Adjacent Code (L1 Summaries)\n${adjacentContext}` : '',
      '',
      '## Target Language',
      spec.targetLanguage,
    ].filter(Boolean).join('\n');
  }

  /**
   * Load L1 summaries of sibling adapters and the composition root.
   * Gives the LLM awareness of existing patterns, constructor signatures,
   * and wiring conventions — critical for consistency in large projects.
   */
  private async loadAdjacentContext(spec: Specification): Promise<string> {
    const sections: string[] = [];

    if (spec.targetAdapter) {
      // Determine which adapter layer (primary or secondary)
      const layer = spec.targetAdapter.includes('primary') ? 'primary' : 'secondary';
      const siblingGlobs = await Promise.all([
        this.fs.glob(`src/adapters/${layer}/**/*.ts`),
        this.fs.glob(`src/adapters/${layer}/**/*.go`),
        this.fs.glob(`src/adapters/${layer}/**/*.rs`),
      ]);
      const siblings = siblingGlobs.flat()
        .filter(f => !f.includes('.test.') && !f.includes('_test.go'));

      // Show up to 5 sibling adapters (L1 — signatures only)
      for (const sib of siblings.slice(0, 5)) {
        try {
          const summary = await this.ast.extractSummary(sib, 'L1');
          const exports = summary.exports
            .map(e => `  ${e.kind} ${e.name}${e.signature ? `: ${e.signature}` : ''}`)
            .join('\n');
          if (exports.trim()) {
            sections.push(`### ${sib} (sibling adapter)\n${exports}`);
          }
        } catch { /* skip */ }
      }
    }

    // Composition root — shows wiring patterns (how adapters are instantiated)
    const crGlobs = await Promise.all([
      this.fs.glob('src/composition-root.*'),
      this.fs.glob('**/composition-root.*'),
      this.fs.glob('**/composition_root.*'),
    ]);
    const crFiles = crGlobs.flat().slice(0, 1);
    for (const cr of crFiles) {
      try {
        const summary = await this.ast.extractSummary(cr, 'L1');
        const exports = summary.exports
          .map(e => `  ${e.kind} ${e.name}${e.signature ? `: ${e.signature}` : ''}`)
          .join('\n');
        if (exports.trim()) {
          sections.push(`### ${cr} (wiring)\n${exports}`);
        }
      } catch { /* skip */ }
    }

    return sections.join('\n\n');
  }

  private async loadPortSummaries(): Promise<ASTSummary[]> {
    const portFiles = (await Promise.all([
      this.fs.glob('src/core/ports/**/*.ts'),
      this.fs.glob('src/core/ports/**/*.go'),
      this.fs.glob('src/core/ports/**/*.rs'),
      this.fs.glob('internal/ports/**/*.go'),
      this.fs.glob('pkg/**/*.go'),
    ])).flat();
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

function langExtension(lang: Language): string {
  if (lang === 'go') return '.go';
  if (lang === 'rust') return '.rs';
  return '.ts';
}

function buildLanguageRules(lang: Language): string {
  if (lang === 'go') {
    return [
      '## Go-Specific Rules',
      '- Exported names MUST start with an uppercase letter (Go visibility convention).',
      '- Port interfaces are Go interfaces. Adapters are structs that satisfy them.',
      '- Use constructor functions (NewXxxAdapter) that return the interface type, not the struct.',
      '- Import paths use Go module paths (e.g., "myproject/core/ports").',
      '- Error handling: return (result, error) tuples, never panic in adapters.',
      '- File naming: snake_case.go (e.g., cache_adapter.go, http_adapter.go).',
      '- Test files: *_test.go in the same package.',
      '- Use context.Context as first parameter for methods that do I/O.',
      '- The composition root is typically main.go or composition_root.go.',
    ].join('\n');
  }

  if (lang === 'rust') {
    return [
      '## Rust-Specific Rules',
      '- Port interfaces are Rust traits. Adapters are structs that impl the trait.',
      '- Use `pub` for exported items. Non-pub items are module-private.',
      '- Import paths use `crate::core::ports::PortName` for internal modules.',
      '- Use `mod.rs` or direct file names for module declarations.',
      '- Error handling: return Result<T, E> with custom error enums. Never unwrap() in adapters.',
      '- File naming: snake_case.rs (e.g., cache_adapter.rs, http_adapter.rs).',
      '- Test modules: `#[cfg(test)] mod tests { ... }` within the same file.',
      '- Use `async fn` with `async-trait` crate for async port interfaces.',
      '- The composition root is typically main.rs or lib.rs.',
      '- Lifetimes: prefer owned types in port interfaces to avoid lifetime complexity.',
    ].join('\n');
  }

  return [
    '## TypeScript-Specific Rules',
    '- Use .js extensions on all relative imports (NodeNext module resolution).',
    '- Import from ports via ../../core/ports/index.js.',
    '- moduleResolution MUST be "nodenext" (not "bundler").',
    '- strict: true, no implicit any.',
    '- Port interfaces use TypeScript `interface` keyword.',
    '- Adapters are classes that `implements` the port interface.',
    '- Constructor injection: `constructor(private readonly port: IMyPort) {}`.',
  ].join('\n');
}
