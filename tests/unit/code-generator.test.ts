import { describe, it, expect } from 'bun:test';
import type {
  ILLMPort,
  IASTPort,
  IBuildPort,
  IFileSystemPort,
  ASTSummary,
  LintError,
  TokenBudget,
  Message,
} from '../../src/core/ports/index.js';
import { CodeGenerator } from '../../src/core/usecases/code-generator.js';

// ─── Mock Factories ─────────────────────────────────────

function mockLLM(response: string): ILLMPort {
  return {
    prompt: async (_b: TokenBudget, _m: Message[]) => ({
      content: response,
      tokenUsage: { input: 100, output: 50 },
      model: 'mock',
    }),
    streamPrompt: async function* () { yield response; },
  };
}

function mockAST(): IASTPort {
  return {
    extractSummary: async (filePath: string, level: ASTSummary['level']): Promise<ASTSummary> => ({
      filePath, language: 'typescript', level,
      exports: [{ name: 'ILLMPort', kind: 'interface' }],
      imports: [], dependencies: [], lineCount: 10, tokenEstimate: 50,
    }),
    diffStructural: () => ({ added: [], removed: [], modified: [] }),
  };
}

function mockBuild(): IBuildPort {
  return {
    compile: async () => ({ success: true, errors: [], duration: 100 }),
    lint: async () => ({ success: true, errors: [], warningCount: 0, errorCount: 0 }),
    test: async () => ({ success: true, passed: 1, failed: 0, skipped: 0, duration: 50, failures: [] }),
  };
}

function mockFS(files: string[]): IFileSystemPort {
  return {
    read: async () => '',
    write: async () => {},
    exists: async () => true,
    glob: async () => files,
  };
}

function capturingLLM(response: string): ILLMPort & { captured: Message[][] } {
  const spy: ILLMPort & { captured: Message[][] } = {
    captured: [],
    prompt: async (_b, msgs) => {
      spy.captured.push(msgs);
      return { content: response, tokenUsage: { input: 100, output: 50 }, model: 'mock' };
    },
    streamPrompt: async function* () { yield response; },
  };
  return spy;
}

// ─── Tests ──────────────────────────────────────────────

const spec = {
  title: 'Auth Adapter',
  requirements: ['JWT validation', 'Token refresh'],
  constraints: ['No external deps'],
  targetLanguage: 'typescript' as const,
};

describe('CodeGenerator.generateFromSpec', () => {
  it('calls llm.prompt with spec requirements in the prompt', async () => {
    const llm = capturingLLM('// generated');
    const gen = new CodeGenerator(llm, mockAST(), mockBuild(), mockFS(['src/core/ports/index.ts']));
    await gen.generateFromSpec(spec, 'typescript');
    const userMsg = llm.captured[0].find((m) => m.role === 'user')!;
    expect(userMsg.content).toContain('JWT validation');
    expect(userMsg.content).toContain('Token refresh');
  });

  it('returns a CodeUnit with the LLM response as content', async () => {
    const gen = new CodeGenerator(mockLLM('const x = 1;'), mockAST(), mockBuild(), mockFS([]));
    const unit = await gen.generateFromSpec(spec, 'typescript');
    expect(unit.content).toBe('const x = 1;');
    expect(unit.language).toBe('typescript');
  });

  it('includes port interface context in the prompt (L1 summaries)', async () => {
    const llm = capturingLLM('// code');
    const gen = new CodeGenerator(llm, mockAST(), mockBuild(), mockFS(['src/core/ports/index.ts']));
    await gen.generateFromSpec(spec, 'typescript');
    const sysMsg = llm.captured[0].find((m) => m.role === 'system')!;
    expect(sysMsg.content).toContain('ILLMPort');
    expect(sysMsg.content).toContain('Port Interfaces');
  });
});

describe('CodeGenerator.refineFromFeedback', () => {
  const unit = {
    filePath: 'src/adapters/auth.ts',
    language: 'typescript' as const,
    content: 'const x: any = 1;',
    astSummary: {
      filePath: 'src/adapters/auth.ts', language: 'typescript' as const,
      level: 'L0' as const, exports: [], imports: [], dependencies: [],
      lineCount: 1, tokenEstimate: 10,
    },
  };

  const lintErrors: LintError[] = [{
    filePath: 'src/adapters/auth.ts', line: 1, column: 10,
    severity: 'error', message: 'Unexpected any', rule: 'no-explicit-any',
  }];

  it('includes lint errors in the refinement prompt', async () => {
    const llm = capturingLLM('const x: number = 1;');
    const gen = new CodeGenerator(llm, mockAST(), mockBuild(), mockFS([]));
    await gen.refineFromFeedback(unit, lintErrors);
    const userMsg = llm.captured[0].find((m) => m.role === 'user')!;
    expect(userMsg.content).toContain('Unexpected any');
    expect(userMsg.content).toContain('no-explicit-any');
  });

  it('returns updated CodeUnit with new content', async () => {
    const gen = new CodeGenerator(mockLLM('const x: number = 1;'), mockAST(), mockBuild(), mockFS([]));
    const result = await gen.refineFromFeedback(unit, lintErrors);
    expect(result.content).toBe('const x: number = 1;');
  });

  it('preserves the file path from the original CodeUnit', async () => {
    const gen = new CodeGenerator(mockLLM('fixed'), mockAST(), mockBuild(), mockFS([]));
    const result = await gen.refineFromFeedback(unit, lintErrors);
    expect(result.filePath).toBe('src/adapters/auth.ts');
  });
});
