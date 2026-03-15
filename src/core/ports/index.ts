/**
 * hex-intf Port Interfaces
 *
 * All communication between domain core and adapters flows through these typed ports.
 * Input ports are implemented by use cases (driven by primary adapters).
 * Output ports are implemented by secondary adapters (driven by use cases).
 */

// ─── Value Objects ───────────────────────────────────────

export type Language = 'typescript' | 'go' | 'rust';

export interface ASTSummary {
  filePath: string;
  language: Language;
  level: 'L0' | 'L1' | 'L2' | 'L3';
  exports: ExportEntry[];
  imports: ImportEntry[];
  dependencies: string[];
  lineCount: number;
  tokenEstimate: number;
  raw?: string; // Only present at L3
}

export interface ExportEntry {
  name: string;
  kind: 'function' | 'class' | 'interface' | 'type' | 'const' | 'enum';
  signature?: string; // Present at L2+
}

export interface ImportEntry {
  names: string[];
  from: string;
}

export interface TokenBudget {
  maxTokens: number;
  reservedForResponse: number;
  available: number;
}

export interface CodeUnit {
  filePath: string;
  language: Language;
  content: string;
  astSummary: ASTSummary;
}

export interface Specification {
  title: string;
  requirements: string[];
  constraints: string[];
  targetLanguage: Language;
  targetAdapter?: string;
}

export interface Workplan {
  id: string;
  title: string;
  steps: WorkplanStep[];
  estimatedTokenBudget: number;
}

export interface WorkplanStep {
  id: string;
  description: string;
  adapter: string;
  dependencies: string[]; // step IDs
  assignee?: string;
}

export interface StepResult {
  stepId: string;
  status: 'pending' | 'running' | 'passed' | 'failed';
  output?: CodeUnit;
  errors?: string[];
}

export interface LintError {
  filePath: string;
  line: number;
  column: number;
  severity: 'error' | 'warning';
  message: string;
  rule: string;
}

export interface BuildResult {
  success: boolean;
  errors: string[];
  duration: number;
}

export interface LintResult {
  success: boolean;
  errors: LintError[];
  warningCount: number;
  errorCount: number;
}

export interface TestResult {
  success: boolean;
  passed: number;
  failed: number;
  skipped: number;
  duration: number;
  failures: TestFailure[];
}

export interface TestFailure {
  testName: string;
  message: string;
  expected?: string;
  actual?: string;
}

export interface StructuralDiff {
  added: ExportEntry[];
  removed: ExportEntry[];
  modified: Array<{ before: ExportEntry; after: ExportEntry }>;
}

export interface WorktreePath {
  absolutePath: string;
  branch: string;
}

export interface MergeResult {
  success: boolean;
  conflicts: string[];
  commitHash?: string;
}

export interface Message {
  role: 'system' | 'user' | 'assistant';
  content: string;
}

export interface LLMResponse {
  content: string;
  tokenUsage: { input: number; output: number };
  model: string;
}

export interface TestSuite {
  name: string;
  filePaths: string[];
  type: 'unit' | 'integration';
}

export interface Project {
  name: string;
  rootPath: string;
  language: Language;
  adapters: string[];
}

// ─── Input Ports (Primary / Driving) ─────────────────────

export interface ICodeGenerationPort {
  generateFromSpec(spec: Specification, lang: Language): Promise<CodeUnit>;
  refineFromFeedback(unit: CodeUnit, errors: LintError[]): Promise<CodeUnit>;
}

export interface IWorkplanPort {
  createPlan(requirements: string[], lang: Language): Promise<Workplan>;
  executePlan(plan: Workplan): AsyncGenerator<StepResult>;
}

export interface ISummaryPort {
  summarizeFile(filePath: string, level: ASTSummary['level']): Promise<ASTSummary>;
  summarizeProject(rootPath: string, level: ASTSummary['level']): Promise<ASTSummary[]>;
}

// ─── Output Ports (Secondary / Driven) ───────────────────

export interface IASTPort {
  extractSummary(filePath: string, level: ASTSummary['level']): Promise<ASTSummary>;
  diffStructural(before: ASTSummary, after: ASTSummary): StructuralDiff;
}

export interface ILLMPort {
  prompt(budget: TokenBudget, messages: Message[]): Promise<LLMResponse>;
  streamPrompt(budget: TokenBudget, messages: Message[]): AsyncGenerator<string>;
}

export interface IBuildPort {
  compile(project: Project): Promise<BuildResult>;
  lint(project: Project): Promise<LintResult>;
  test(project: Project, suite: TestSuite): Promise<TestResult>;
}

export interface IWorktreePort {
  create(branchName: string): Promise<WorktreePath>;
  merge(worktree: WorktreePath, target: string): Promise<MergeResult>;
  cleanup(worktree: WorktreePath): Promise<void>;
  list(): Promise<WorktreePath[]>;
}

export interface IGitPort {
  commit(message: string): Promise<string>;
  createBranch(name: string): Promise<void>;
  diff(base: string, head: string): Promise<string>;
  currentBranch(): Promise<string>;
}

export interface IFileSystemPort {
  read(filePath: string): Promise<string>;
  write(filePath: string, content: string): Promise<void>;
  exists(filePath: string): Promise<boolean>;
  glob(pattern: string): Promise<string[]>;
}

// ─── Analysis Ports ──────────────────────────────────────

export type DependencyDirection = 'domain' | 'ports' | 'usecases' | 'adapters/primary' | 'adapters/secondary' | 'infrastructure';

export interface ImportEdge {
  from: string;       // file path
  to: string;         // file path
  names: string[];    // imported symbols
}

export interface DeadExport {
  filePath: string;
  exportName: string;
  kind: ExportEntry['kind'];
}

export interface DependencyViolation {
  from: string;       // file path
  to: string;         // file path
  fromLayer: DependencyDirection;
  toLayer: DependencyDirection;
  rule: string;       // which hex rule is broken
}

export interface ArchAnalysisResult {
  deadExports: DeadExport[];
  orphanFiles: string[];
  dependencyViolations: DependencyViolation[];
  circularDeps: string[][];  // each array is a cycle
  unusedPorts: string[];     // port interface names with no adapter
  unusedAdapters: string[];  // adapter files implementing unused ports
  summary: {
    totalFiles: number;
    totalExports: number;
    deadExportCount: number;
    violationCount: number;
    circularCount: number;
    healthScore: number;      // 0-100, penalized by violations
  };
}

export interface IArchAnalysisPort {
  /** Build the full import/export dependency graph from L1 summaries */
  buildDependencyGraph(rootPath: string): Promise<ImportEdge[]>;

  /** Find exports that no other file imports */
  findDeadExports(rootPath: string): Promise<DeadExport[]>;

  /** Validate hexagonal dependency direction rules */
  validateHexBoundaries(rootPath: string): Promise<DependencyViolation[]>;

  /** Detect circular import chains */
  detectCircularDeps(rootPath: string): Promise<string[][]>;

  /** Full analysis: dead code + hex validation + circular detection */
  analyzeArchitecture(rootPath: string): Promise<ArchAnalysisResult>;
}
