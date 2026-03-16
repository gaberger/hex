/**
 * Build secondary adapter -- implements IBuildPort.
 *
 * Runs compile, lint, and test commands for TypeScript projects using
 * Bun/tsc/eslint via execFile (no shell injection). Designed for future
 * extension to Go and Rust.
 */
import { execFile as execFileCb } from 'node:child_process';
import { promisify } from 'node:util';
import type {
  BuildResult,
  IBuildPort,
  Language,
  LintError,
  LintResult,
  Project,
  TestFailure,
  TestResult,
  TestSuite,
} from '../../core/ports/index.js';

const execFile = promisify(execFileCb);

class BuildError extends Error {
  override readonly name = 'BuildError';
  constructor(message: string, public readonly command: string) {
    super(message);
  }
}

export class BuildAdapter implements IBuildPort {
  constructor(
    private readonly projectPath: string,
    private readonly language: Language = 'typescript',
  ) {}

  /** The language this adapter was configured for. */
  get configuredLanguage(): Language {
    return this.language;
  }

  async compile(project: Project): Promise<BuildResult> {
    const start = Date.now();
    try {
      await this.run('npx', ['tsc', '--noEmit'], project.rootPath);
      return { success: true, errors: [], duration: Date.now() - start };
    } catch (err: unknown) {
      const e = err as { stdout?: string; stderr?: string };
      const output = (e.stdout ?? '') + (e.stderr ?? '');
      const errors = output.split('\n').filter((l) => /error TS\d+/.test(l));
      return { success: false, errors, duration: Date.now() - start };
    }
  }

  async lint(project: Project): Promise<LintResult> {
    try {
      await this.run('npx', ['eslint', '--format', 'json', '.'], project.rootPath);
      return { success: true, errors: [], warningCount: 0, errorCount: 0 };
    } catch (err: unknown) {
      const e = err as { stdout?: string };
      return this.parseEslintOutput(e.stdout ?? '[]');
    }
  }

  async test(project: Project, suite: TestSuite): Promise<TestResult> {
    const args = ['test', ...suite.filePaths];
    const start = Date.now();
    try {
      const { stdout } = await this.run('bun', args, project.rootPath);
      return this.parseTestOutput(stdout, Date.now() - start);
    } catch (err: unknown) {
      const e = err as { stdout?: string };
      return this.parseTestOutput(e.stdout ?? '', Date.now() - start);
    }
  }

  // ── Private helpers ─────────────────────────────────────────

  private parseEslintOutput(raw: string): LintResult {
    try {
      const parsed: unknown = JSON.parse(raw);
      if (!Array.isArray(parsed)) {
        return { success: false, errors: [], warningCount: 0, errorCount: 1 };
      }
      const files = parsed as Array<{
        filePath: string;
        messages: Array<{
          line: number; column: number;
          severity: number; message: string; ruleId: string | null;
        }>;
      }>;
      const errors: LintError[] = [];
      let warningCount = 0;
      let errorCount = 0;
      for (const file of files) {
        for (const msg of file.messages) {
          const severity = msg.severity === 2 ? 'error' as const : 'warning' as const;
          if (severity === 'error') errorCount++;
          else warningCount++;
          errors.push({
            filePath: file.filePath,
            line: msg.line,
            column: msg.column,
            severity,
            message: msg.message,
            rule: msg.ruleId ?? 'unknown',
          });
        }
      }
      return { success: errorCount === 0, errors, warningCount, errorCount };
    } catch (e) {
      // ESLint output may not be valid JSON (e.g., eslint crashed or produced text errors)
      console.error('Warning: failed to parse eslint output:', e);
      return { success: false, errors: [], warningCount: 0, errorCount: 1 };
    }
  }

  private parseTestOutput(stdout: string, duration: number): TestResult {
    const passMatch = stdout.match(/(\d+)\s+pass/i);
    const failMatch = stdout.match(/(\d+)\s+fail/i);
    const skipMatch = stdout.match(/(\d+)\s+skip/i);
    const passed = passMatch ? Number(passMatch[1]) : 0;
    const failed = failMatch ? Number(failMatch[1]) : 0;
    const skipped = skipMatch ? Number(skipMatch[1]) : 0;

    const failures: TestFailure[] = [];
    const failBlocks = stdout.split(/(?=FAIL|fail\s)/);
    for (const block of failBlocks) {
      const nameMatch = block.match(/(?:FAIL|fail)\s+(.+)/);
      const msgMatch = block.match(/(?:error|Error):\s*(.+)/);
      if (nameMatch) {
        failures.push({
          testName: nameMatch[1].trim(),
          message: msgMatch?.[1]?.trim() ?? 'Unknown failure',
        });
      }
    }

    return { success: failed === 0, passed, failed, skipped, duration, failures };
  }

  private async run(
    cmd: string,
    args: string[],
    cwd?: string,
  ): Promise<{ stdout: string; stderr: string }> {
    return execFile(cmd, args, {
      cwd: cwd ?? this.projectPath,
      maxBuffer: 10 * 1024 * 1024,
    });
  }
}
