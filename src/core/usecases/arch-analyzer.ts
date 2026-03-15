/**
 * Architecture Analyzer Use Case
 *
 * Implements IArchAnalysisPort by composing IASTPort and IFileSystemPort.
 * Performs dead-export detection, hex boundary validation, and circular
 * dependency detection using L1 AST summaries.
 */

import type {
  IASTPort,
  IFileSystemPort,
  IArchAnalysisPort,
  ImportEdge,
  DeadExport,
  DependencyViolation,
  ArchAnalysisResult,
  ASTSummary,
  DependencyDirection,
} from '../ports/index.js';
import { classifyLayer, getViolationRule } from './layer-classifier.js';
import { resolveImportPath, normalizePath } from './path-normalizer.js';

const ENTRY_POINTS = ['index.ts', 'cli.ts', 'main.ts'];

function matchesExclude(filePath: string, patterns: string[]): boolean {
  return patterns.some((p) => {
    if (p.startsWith('*')) return filePath.endsWith(p.slice(1));
    return filePath.includes(p);
  });
}

function isEntryPoint(filePath: string): boolean {
  return ENTRY_POINTS.some((ep) => filePath.endsWith(`/${ep}`) || filePath === ep);
}

function hasReExports(summary: ASTSummary): boolean {
  // A re-export file primarily re-exports — it has exports but its imports
  // reference the same names it exports.
  const exportNames = new Set(summary.exports.map((e) => e.name));
  const importNames = new Set(summary.imports.flatMap((i) => i.names));
  if (exportNames.size === 0) return false;
  let reExportCount = 0;
  for (const name of exportNames) {
    if (importNames.has(name)) reExportCount++;
  }
  return reExportCount / exportNames.size > 0.5;
}

export class ArchAnalyzer implements IArchAnalysisPort {
  constructor(
    private readonly ast: IASTPort,
    private readonly fs: IFileSystemPort,
    private readonly excludePatterns: string[] = [
      'node_modules', 'dist', '*.test.ts', '*.spec.ts',
    ],
  ) {}

  private async collectSummaries(): Promise<ASTSummary[]> {
    const allFiles = await this.fs.glob('**/*.ts');
    const sourceFiles = allFiles.filter((f) => !matchesExclude(f, this.excludePatterns));
    return Promise.all(
      sourceFiles.map((f) => this.ast.extractSummary(f, 'L1')),
    );
  }

  async buildDependencyGraph(_rootPath: string): Promise<ImportEdge[]> {
    const summaries = await this.collectSummaries();
    const edges: ImportEdge[] = [];

    for (const summary of summaries) {
      const fromFile = normalizePath(summary.filePath);
      for (const imp of summary.imports) {
        edges.push({
          from: fromFile,
          to: resolveImportPath(summary.filePath, imp.from),
          names: imp.names,
        });
      }
    }

    return edges;
  }

  async findDeadExports(_rootPath: string): Promise<DeadExport[]> {
    const summaries = await this.collectSummaries();

    // Build set of all imported (name, normalizedTarget) tuples
    const importedByModule = new Map<string, Set<string>>();
    for (const summary of summaries) {
      for (const imp of summary.imports) {
        const target = resolveImportPath(summary.filePath, imp.from);
        if (!importedByModule.has(target)) importedByModule.set(target, new Set());
        for (const name of imp.names) {
          importedByModule.get(target)!.add(name);
        }
      }
    }

    const dead: DeadExport[] = [];
    for (const summary of summaries) {
      const normalizedFile = normalizePath(summary.filePath);
      if (isEntryPoint(normalizedFile)) continue;
      if (hasReExports(summary)) continue;

      const importedFromThis = importedByModule.get(normalizedFile) ?? new Set();
      for (const exp of summary.exports) {
        if (!importedFromThis.has(exp.name)) {
          dead.push({
            filePath: normalizedFile,
            exportName: exp.name,
            kind: exp.kind,
          });
        }
      }
    }

    return dead;
  }

  async validateHexBoundaries(_rootPath: string): Promise<DependencyViolation[]> {
    const edges = await this.buildDependencyGraph('');
    const violations: DependencyViolation[] = [];

    for (const edge of edges) {
      const fromLayer = classifyLayer(edge.from);
      const toLayer = classifyLayer(edge.to);

      if (fromLayer === 'unknown' || toLayer === 'unknown') continue;

      const rule = getViolationRule(
        fromLayer as DependencyDirection,
        toLayer as DependencyDirection,
      );
      if (rule !== null) {
        violations.push({
          from: edge.from,
          to: edge.to,
          fromLayer: fromLayer as DependencyDirection,
          toLayer: toLayer as DependencyDirection,
          rule,
        });
      }
    }

    return violations;
  }

  async detectCircularDeps(_rootPath: string): Promise<string[][]> {
    const edges = await this.buildDependencyGraph('');

    // Build adjacency list
    const graph = new Map<string, Set<string>>();
    for (const edge of edges) {
      if (!graph.has(edge.from)) graph.set(edge.from, new Set());
      graph.get(edge.from)!.add(edge.to);
    }

    const cycles: string[][] = [];
    const visited = new Set<string>();
    const inStack = new Set<string>();
    const stack: string[] = [];

    const dfs = (node: string): void => {
      visited.add(node);
      inStack.add(node);
      stack.push(node);

      const neighbors = graph.get(node);
      if (neighbors) {
        for (const neighbor of neighbors) {
          if (!visited.has(neighbor)) {
            dfs(neighbor);
          } else if (inStack.has(neighbor)) {
            const cycleStart = stack.indexOf(neighbor);
            cycles.push(stack.slice(cycleStart));
          }
        }
      }

      stack.pop();
      inStack.delete(node);
    };

    for (const node of graph.keys()) {
      if (!visited.has(node)) {
        dfs(node);
      }
    }

    return cycles;
  }

  async analyzeArchitecture(_rootPath: string): Promise<ArchAnalysisResult> {
    const summaries = await this.collectSummaries();
    const edges = await this.buildDependencyGraph('');

    const [deadExports, violations, circularDeps] = await Promise.all([
      this.findDeadExports(''),
      this.validateHexBoundaries(''),
      this.detectCircularDeps(''),
    ]);

    // Orphan files: no incoming or outgoing edges
    const connected = new Set<string>();
    for (const edge of edges) {
      connected.add(edge.from);
      connected.add(edge.to);
    }
    const orphanFiles = summaries
      .map((s) => normalizePath(s.filePath))
      .filter((f) => !connected.has(f));

    const totalExports = summaries.reduce((sum, s) => sum + s.exports.length, 0);

    // Health score
    let healthScore = 100;
    healthScore -= violations.length * 5;
    healthScore -= deadExports.length * 2;
    healthScore -= circularDeps.length * 10;
    healthScore = Math.max(0, Math.min(100, healthScore));

    return {
      deadExports,
      orphanFiles,
      dependencyViolations: violations,
      circularDeps,
      unusedPorts: [],   // Requires L2 port interface analysis (future)
      unusedAdapters: [], // Requires L2 port interface analysis (future)
      summary: {
        totalFiles: summaries.length,
        totalExports,
        deadExportCount: deadExports.length,
        violationCount: violations.length,
        circularCount: circularDeps.length,
        healthScore,
      },
    };
  }
}
