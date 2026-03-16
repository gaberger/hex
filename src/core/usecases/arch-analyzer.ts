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

const ENTRY_POINTS = ['index.ts', 'cli.ts', 'main.ts', 'composition-root.ts', 'main.go', 'main.rs'];

/** Glob patterns for all supported framework languages (ts, go, rust). */
const SOURCE_GLOBS = ['**/*.ts', '**/*.go', '**/*.rs'];

/** Exported functions that serve as entry points (not dead despite no importers) */
const ENTRY_EXPORTS = new Set(['runCLI', 'startDashboard', 'createAppContext']);

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
      'node_modules', 'dist', 'examples',
      '*.test.ts', '*.spec.ts',   // TypeScript tests
      '*_test.go',                 // Go tests
      '*.test.rs',                 // Rust tests
    ],
  ) {}

  private async collectSummaries(): Promise<ASTSummary[]> {
    const results = await Promise.all(SOURCE_GLOBS.map((g) => this.fs.glob(g)));
    const allFiles = results.flat();
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
        if (!importedFromThis.has(exp.name) && !ENTRY_EXPORTS.has(exp.name)) {
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
    // Collect summaries ONCE and pass to all sub-analyses to avoid 5x re-parsing
    const summaries = await this.collectSummaries();
    const edges = this.buildEdgesFromSummaries(summaries);

    const deadExports = this.findDeadFromSummaries(summaries);
    const violations = this.findViolationsFromEdges(edges);
    const circularDeps = this.findCyclesFromEdges(edges);

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

    // Detect unused ports: port interfaces with no adapter implementing them
    const { unusedPorts, unusedAdapters } = this.detectUnusedPorts(summaries);

    // Health score — violations and cycles are severe, dead exports minor
    let healthScore = 100;
    healthScore -= violations.length * 10;
    healthScore -= circularDeps.length * 15;
    healthScore -= Math.min(20, deadExports.length * 1);  // cap dead export penalty at 20
    healthScore -= Math.min(10, unusedPorts.length * 1);   // cap unused port penalty at 10
    healthScore = Math.max(0, Math.min(100, healthScore));

    return {
      deadExports,
      orphanFiles,
      dependencyViolations: violations,
      circularDeps,
      unusedPorts,
      unusedAdapters,
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

  // ── Internal methods that operate on pre-collected summaries ──────

  private buildEdgesFromSummaries(summaries: ASTSummary[]): ImportEdge[] {
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

  private findDeadFromSummaries(summaries: ASTSummary[]): DeadExport[] {
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
        if (!importedFromThis.has(exp.name) && !ENTRY_EXPORTS.has(exp.name)) {
          dead.push({ filePath: normalizedFile, exportName: exp.name, kind: exp.kind });
        }
      }
    }
    return dead;
  }

  private findViolationsFromEdges(edges: ImportEdge[]): DependencyViolation[] {
    const violations: DependencyViolation[] = [];
    for (const edge of edges) {
      const fromLayer = classifyLayer(edge.from);
      const toLayer = classifyLayer(edge.to);
      if (fromLayer === 'unknown' || toLayer === 'unknown') continue;
      const rule = getViolationRule(fromLayer as DependencyDirection, toLayer as DependencyDirection);
      if (rule !== null) {
        violations.push({
          from: edge.from, to: edge.to,
          fromLayer: fromLayer as DependencyDirection,
          toLayer: toLayer as DependencyDirection,
          rule,
        });
      }
    }
    return violations;
  }

  private findCyclesFromEdges(edges: ImportEdge[]): string[][] {
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
      visited.add(node); inStack.add(node); stack.push(node);
      const neighbors = graph.get(node);
      if (neighbors) {
        for (const neighbor of neighbors) {
          if (!visited.has(neighbor)) dfs(neighbor);
          else if (inStack.has(neighbor)) cycles.push(stack.slice(stack.indexOf(neighbor)));
        }
      }
      stack.pop(); inStack.delete(node);
    };
    for (const node of graph.keys()) {
      if (!visited.has(node)) dfs(node);
    }
    return cycles;
  }

  /**
   * Detect port interfaces that have no adapter implementation.
   *
   * Strategy: collect all interface exports from ports/ files whose names
   * start with 'I' and end with 'Port'. Then check if any adapter file
   * imports that interface name — if not, the port is unused.
   */
  private detectUnusedPorts(summaries: ASTSummary[]): {
    unusedPorts: string[];
    unusedAdapters: string[];
  } {
    // Collect port interface names from port files
    const portInterfaces = new Set<string>();
    for (const s of summaries) {
      const normalized = normalizePath(s.filePath);
      if (!normalized.includes('/ports/')) continue;
      for (const exp of s.exports) {
        if (exp.kind === 'interface' && exp.name.startsWith('I') && exp.name.endsWith('Port')) {
          portInterfaces.add(exp.name);
        }
      }
    }

    // Collect all interface names imported by adapter files
    const implementedPorts = new Set<string>();
    const adapterFiles: string[] = [];
    for (const s of summaries) {
      const normalized = normalizePath(s.filePath);
      if (!normalized.includes('/adapters/')) continue;
      adapterFiles.push(normalized);
      for (const imp of s.imports) {
        for (const name of imp.names) {
          if (portInterfaces.has(name)) {
            implementedPorts.add(name);
          }
        }
      }
    }

    const unusedPorts = Array.from(portInterfaces).filter(
      (name) => !implementedPorts.has(name),
    );

    // Adapter files that don't import any port interface are potentially unused
    const unusedAdapters = adapterFiles.filter((f) => {
      const s = summaries.find((s) => normalizePath(s.filePath) === f);
      if (!s) return false;
      const importedPorts = s.imports.flatMap((i) => i.names).filter((n) => portInterfaces.has(n));
      return importedPorts.length === 0;
    });

    return { unusedPorts, unusedAdapters };
  }
}
