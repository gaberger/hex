/**
 * Summary Service use case -- implements ISummaryPort.
 *
 * Provides file and project-level AST summaries by delegating
 * to the IASTPort and IFileSystemPort.
 */
import type {
  ASTSummary,
  IASTPort,
  IFileSystemPort,
  ISummaryPort,
} from '../ports/index.js';

const EXCLUDED_PATTERNS = ['node_modules', 'dist', '.test.', '.spec.', '_test.go'];

export class SummaryService implements ISummaryPort {
  constructor(
    private readonly ast: IASTPort,
    private readonly fs: IFileSystemPort,
  ) {}

  async summarizeFile(
    filePath: string,
    level: ASTSummary['level'],
  ): Promise<ASTSummary> {
    return this.ast.extractSummary(filePath, level);
  }

  async summarizeProject(
    rootPath: string,
    level: ASTSummary['level'],
  ): Promise<ASTSummary[]> {
    const globResults = await Promise.all([
      this.fs.glob(`${rootPath}/**/*.ts`),
      this.fs.glob(`${rootPath}/**/*.go`),
      this.fs.glob(`${rootPath}/**/*.rs`),
    ]);
    const allFiles = globResults.flat();
    const sourceFiles = allFiles.filter(
      (f) => !EXCLUDED_PATTERNS.some((p) => f.includes(p)),
    );

    const summaries: ASTSummary[] = [];
    for (const file of sourceFiles) {
      try {
        const summary = await this.ast.extractSummary(file, level);
        summaries.push(summary);
      } catch {
        // Skip files that cannot be parsed
      }
    }
    return summaries;
  }
}
