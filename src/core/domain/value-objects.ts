/**
 * Domain Value Objects
 *
 * Shared value types used by both domain entities and port interfaces.
 * This file is the canonical source — ports re-export these types to
 * keep the public API stable, but the ownership is in the domain layer.
 *
 * Dependency direction:
 *   value-objects.ts (domain) <- entities.ts (domain)
 *   value-objects.ts (domain) <- ports/index.ts (re-exports)
 */

// ─── Language & AST ─────────────────────────────────────

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
  stubbed?: boolean; // true when tree-sitter is unavailable
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

// ─── Token & Code ───────────────────────────────────────

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

// ─── Specification & Workplan ───────────────────────────

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

// ─── Build & Lint ───────────────────────────────────────

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

// ─── Testing ────────────────────────────────────────────

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

export interface TestSuite {
  name: string;
  filePaths: string[];
  type: 'unit' | 'integration';
}

// ─── Structural Diff ────────────────────────────────────

export interface StructuralDiff {
  added: ExportEntry[];
  removed: ExportEntry[];
  modified: Array<{ before: ExportEntry; after: ExportEntry }>;
}

// ─── Git & Worktree ─────────────────────────────────────

export interface WorktreePath {
  absolutePath: string;
  branch: string;
}

export interface MergeResult {
  success: boolean;
  conflicts: string[];
  commitHash?: string;
}

// ─── LLM ────────────────────────────────────────────────

export interface Message {
  role: 'system' | 'user' | 'assistant';
  content: string;
}

// ─── Project Registry ──────────────────────────────────

export interface ProjectRegistration {
  id: string;
  name: string;
  rootPath: string;
  port: number;
  status: 'active' | 'inactive' | 'error';
  createdAt: number;
  lastSeenAt: number;
}

export interface ProjectRegistry {
  version: 1;
  projects: ProjectRegistration[];
}

export interface LocalProjectIdentity {
  id: string;
  name: string;
  createdAt: number;
}

export interface LLMResponse {
  content: string;
  tokenUsage: { input: number; output: number };
  model: string;
}

// ─── Project ────────────────────────────────────────────

export interface Project {
  name: string;
  rootPath: string;
  language: Language;
  adapters: string[];
}

// ─── Analysis Types ─────────────────────────────────────

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
