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
  IGitPort,
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
import { analyzeRepoHygiene } from '../domain/repo-hygiene.js';
import type { GitStateSnapshot } from '../domain/repo-hygiene.js';

const ENTRY_POINTS = [
  'index.ts', 'cli.ts', 'main.ts', 'composition-root.ts',  // TypeScript
  'main.go', 'cmd/main.go', 'composition-root.go',          // Go
  'main.rs', 'lib.rs',                                       // Rust
];

/** Glob patterns for all supported framework languages (ts, go, rust). */
const SOURCE_GLOBS = ['**/*.ts', '**/*.go', '**/*.rs'];

/** Exported functions that serve as entry points (not dead despite no importers) */
const ENTRY_EXPORTS = new Set([
  'runCLI', 'startDashboard', 'createAppContext',  // TypeScript
  'main', 'init',                                    // Go: main() and init() are always entry points
  'Main',                                            // Go: exported Main is sometimes used
]);

function matchesExclude(filePath: string, patterns: string[]): boolean {
  return patterns.some((p) => {
    if (p.startsWith('*')) return filePath.endsWith(p.slice(1));
    return filePath.includes(p);
  });
}

function isEntryPoint(filePath: string): boolean {
  return ENTRY_POINTS.some((ep) => filePath.endsWith(`/${ep}`) || filePath === ep)
    || /\/cmd\/[^/]+\/main\.go$/.test(filePath)   // Go: cmd/appname/main.go
    || /\/src\/bin\/[^/]+\.rs$/.test(filePath);    // Rust: src/bin/appname.rs
}

/**
 * Layer-based dead-export skip rules.
 *
 * Ports and adapters are consumed via composition-root DI (often dynamic import),
 * making their exports invisible to static import tracing. Rather than annotating
 * every adapter class, we leverage hex's layer classification to skip dead-export
 * checks for layers where dynamic consumption is the norm.
 *
 * Strict checking remains for domain and usecases — these should have static consumers.
 */
function shouldSkipDeadExportCheck(filePath: string): boolean {
  const layer = classifyLayer(filePath);

  // Ports are contracts — they ARE the public API. Never flag as dead.
  if (layer === 'ports') return true;

  // Adapters are wired by composition-root via dynamic import().
  // Their class/function exports are consumed at runtime, not via static imports.
  if (layer === 'adapters/primary' || layer === 'adapters/secondary') return true;

  // Rust FFI boundaries (NAPI modules, daemon, extractors).
  // These exports are consumed from TypeScript via native bindings, not static imports.
  if (filePath.includes('hex-core/') || filePath.includes('hex-hub/')) return true;
  if (filePath.includes('src/extractors/') && filePath.endsWith('.rs')) return true;

  // Domain, usecases, infrastructure: DO check for dead exports
  return false;
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
  private goModulePrefix: string | null = null;

  /**
   * Normalize a user-supplied scope path into a relative prefix for glob patterns.
   * Absolute paths, '.', and '' all resolve to '' (no prefix = project root).
   */
  private static normalizeScopePrefix(scopePath: string): string {
    if (scopePath === '' || scopePath === '.') return '';
    // Absolute paths: treat as project root (the fs adapter resolves from cwd)
    if (scopePath.startsWith('/')) return '';
    return scopePath.replace(/\/$/, '') + '/';
  }

  constructor(
    private readonly ast: IASTPort,
    private readonly fs: IFileSystemPort,
    private readonly git?: IGitPort,
    private readonly excludePatterns: string[] = [
      'node_modules', 'dist', 'examples',
      '*.test.ts', '*.spec.ts',   // TypeScript tests
      '*_test.go',                 // Go tests
      '*.test.rs',                 // Rust tests
      'tests/',                    // Rust integration test directory
      'target/',                   // Rust/Cargo build artifacts
    ],
  ) {}

  /**
   * Detect Go module name from go.mod in common locations.
   * Returns the module path (e.g. "hex-f1" or "github.com/org/repo") or null.
   */
  private async detectGoModulePrefix(scopePath: string): Promise<string | null> {
    const prefix = ArchAnalyzer.normalizeScopePrefix(scopePath);
    for (const candidate of ['go.mod', 'backend/go.mod', 'src/go.mod', 'cmd/go.mod']) {
      try {
        const content = await this.fs.read(prefix + candidate);
        const match = content.match(/^module\s+(\S+)/m);
        if (match) return match[1];
      } catch { /* not found */ }
    }
    return null;
  }

  private async collectSummaries(scopePath: string): Promise<ASTSummary[]> {
    // Auto-detect Go module prefix once per analysis run
    if (this.goModulePrefix === null) {
      this.goModulePrefix = await this.detectGoModulePrefix(scopePath) ?? '';
    }
    const prefix = ArchAnalyzer.normalizeScopePrefix(scopePath);
    const results = await Promise.all(SOURCE_GLOBS.map((g) => this.fs.glob(prefix + g)));
    const allFiles = results.flat();
    // When scoped to a subdirectory, don't exclude files that are inside the scope
    // (e.g. scoping to 'examples/weather/backend' should not be blocked by the 'examples' exclude)
    const activeExcludes = prefix
      ? this.excludePatterns.filter((p) => !prefix.startsWith(p) && !prefix.includes(`/${p}/`))
      : this.excludePatterns;
    const sourceFiles = allFiles.filter((f) => !matchesExclude(f, activeExcludes));
    return Promise.all(
      sourceFiles.map((f) => this.ast.extractSummary(f, 'L1')),
    );
  }

  async buildDependencyGraph(rootPath: string): Promise<ImportEdge[]> {
    const summaries = await this.collectSummaries(rootPath);
    return this.buildEdgesFromSummaries(summaries);
  }

  async findDeadExports(rootPath: string): Promise<DeadExport[]> {
    const summaries = await this.collectSummaries(rootPath);
    return this.findDeadFromSummaries(summaries);
  }

  async validateHexBoundaries(rootPath: string): Promise<DependencyViolation[]> {
    const edges = await this.buildDependencyGraph(rootPath);
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

  async detectCircularDeps(rootPath: string): Promise<string[][]> {
    const edges = await this.buildDependencyGraph(rootPath);

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

  async analyzeArchitecture(rootPath: string): Promise<ArchAnalysisResult> {
    // Reset Go module prefix for fresh detection on each analysis run
    this.goModulePrefix = null;
    // Collect summaries ONCE and pass to all sub-analyses to avoid 5x re-parsing
    const summaries = await this.collectSummaries(rootPath);
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

    // Repo hygiene (anti-slop) — optional, only when git port is available
    let repoHygiene;
    if (this.git) {
      try {
        const snapshot = await this.collectGitSnapshot(rootPath);
        repoHygiene = analyzeRepoHygiene(snapshot);
        // Penalize health score for hygiene issues
        if (repoHygiene.embeddedRepoCount > 0) healthScore -= repoHygiene.embeddedRepoCount * 5;
        if (repoHygiene.orphanWorktreeCount > 0) healthScore -= repoHygiene.orphanWorktreeCount * 3;
        healthScore = Math.max(0, Math.min(100, healthScore));
      } catch {
        // Git not available — skip hygiene check
      }
    }

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
      repoHygiene,
    };
  }

  // ── Internal methods that operate on pre-collected summaries ──────

  private buildEdgesFromSummaries(summaries: ASTSummary[]): ImportEdge[] {
    const goMod = this.goModulePrefix || undefined;
    const edges: ImportEdge[] = [];
    for (const summary of summaries) {
      const fromFile = normalizePath(summary.filePath);
      for (const imp of summary.imports) {
        edges.push({
          from: fromFile,
          to: resolveImportPath(summary.filePath, imp.from, goMod),
          names: imp.names,
        });
      }
    }
    return edges;
  }

  private findDeadFromSummaries(summaries: ASTSummary[]): DeadExport[] {
    const goMod = this.goModulePrefix || undefined;
    const importedByModule = new Map<string, Set<string>>();

    // Step 1: Build direct import map
    for (const summary of summaries) {
      for (const imp of summary.imports) {
        const target = resolveImportPath(summary.filePath, imp.from, goMod);
        if (!importedByModule.has(target)) importedByModule.set(target, new Set());
        for (const name of imp.names) {
          importedByModule.get(target)!.add(name);
        }
      }
    }

    // Step 2: Build re-export chain map
    // reExportChain[reExporterFile][exportName] = originalSourceFile
    const reExportChain = new Map<string, Map<string, string>>();
    for (const summary of summaries) {
      const normalizedFile = normalizePath(summary.filePath);
      if (!hasReExports(summary)) continue;

      const exportNames = new Set(summary.exports.map((e) => e.name));
      for (const imp of summary.imports) {
        const sourceFile = resolveImportPath(summary.filePath, imp.from, goMod);
        for (const name of imp.names) {
          // This file imports 'name' and also exports 'name' — it's a re-export
          if (exportNames.has(name) || imp.names.includes('*')) {
            if (!reExportChain.has(normalizedFile)) {
              reExportChain.set(normalizedFile, new Map());
            }
            if (imp.names.includes('*')) {
              // export * from 'source' — mark all exports from source as re-exported
              reExportChain.get(normalizedFile)!.set('*', sourceFile);
            } else {
              reExportChain.get(normalizedFile)!.set(name, sourceFile);
            }
          }
        }
      }
    }

    // Step 3: Transitively mark re-exported symbols as used in original files
    const transitiveUsage = new Map<string, Set<string>>();
    for (const [reExporter, chain] of reExportChain) {
      const importedFromReExporter = importedByModule.get(reExporter) ?? new Set();
      for (const importedName of importedFromReExporter) {
        // Someone imports 'importedName' from the re-exporter
        // Find the original source file for this name
        const originalSource = chain.get(importedName) || chain.get('*');
        if (originalSource) {
          if (!transitiveUsage.has(originalSource)) {
            transitiveUsage.set(originalSource, new Set());
          }
          transitiveUsage.get(originalSource)!.add(importedName);
        }
      }
      // If the re-exporter itself is imported with '*', mark all re-exports as used
      if (importedFromReExporter.has('*')) {
        for (const [name, source] of chain) {
          if (!transitiveUsage.has(source)) {
            transitiveUsage.set(source, new Set());
          }
          transitiveUsage.get(source)!.add(name === '*' ? '*' : name);
        }
      }
    }

    // Step 4: Find dead exports, considering both direct and transitive usage
    const dead: DeadExport[] = [];
    for (const summary of summaries) {
      const normalizedFile = normalizePath(summary.filePath);
      if (isEntryPoint(normalizedFile)) continue;
      if (hasReExports(summary)) continue;

      // Layer 2: Skip dead-export checks for layers whose exports are consumed
      // dynamically (adapters wired by composition-root, ports as public contracts).
      if (shouldSkipDeadExportCheck(normalizedFile)) continue;

      const importedFromThis = importedByModule.get(normalizedFile) ?? new Set();
      const transitiveFromThis = transitiveUsage.get(normalizedFile) ?? new Set();

      // If any importer uses '*' (dynamic import without destructuring),
      // treat ALL exports from this module as used
      if (importedFromThis.has('*') || transitiveFromThis.has('*')) continue;

      for (const exp of summary.exports) {
        const isDirectlyUsed = importedFromThis.has(exp.name);
        const isTransitivelyUsed = transitiveFromThis.has(exp.name);
        const isEntryExport = ENTRY_EXPORTS.has(exp.name);

        if (!isDirectlyUsed && !isTransitivelyUsed && !isEntryExport) {
          // Layer 3: Check for @hex:public annotation on the export
          if (exp.annotations?.includes('hex:public')) continue;
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

    // Collect all interface names imported by adapter and use-case files
    // Use cases can also implement ports (e.g., SummaryService implements ISummaryPort)
    const implementedPorts = new Set<string>();
    const adapterFiles: string[] = [];
    for (const s of summaries) {
      const normalized = normalizePath(s.filePath);
      const isAdapter = normalized.includes('/adapters/');
      const isUseCase = normalized.includes('/usecases/');
      if (!isAdapter && !isUseCase) continue;
      if (isAdapter) adapterFiles.push(normalized);
      for (const imp of s.imports) {
        for (const name of imp.names) {
          if (portInterfaces.has(name)) {
            implementedPorts.add(name);
          }
        }
      }
    }

    // Go structural interface matching: check if adapter methods match port interface methods
    const portMethodSets = new Map<string, Set<string>>(); // portName -> method names
    for (const s of summaries) {
      if (!normalizePath(s.filePath).includes('/ports/')) continue;
      for (const exp of s.exports) {
        if (exp.kind === 'interface' && exp.name.startsWith('I') && exp.name.endsWith('Port')) {
          // Collect method names from the interface signature
          if (exp.signature) {
            const methods = new Set<string>();
            // Simple heuristic: look for method-like patterns in the signature
            const methodMatches = exp.signature.matchAll(/\b([A-Z]\w+)\s*\(/g);
            for (const m of methodMatches) {
              methods.add(m[1]);
            }
            if (methods.size > 0) {
              portMethodSets.set(exp.name, methods);
            }
          }
        }
      }
    }

    // Check adapter structs for method name overlap with port interfaces
    for (const s of summaries) {
      const normalized = normalizePath(s.filePath);
      if (!normalized.includes('/adapters/') || !normalized.endsWith('.go')) continue;
      const adapterMethods = new Set(
        s.exports.filter((e) => e.kind === 'function').map((e) => e.name),
      );
      for (const [portName, portMethods] of portMethodSets) {
        if (implementedPorts.has(portName)) continue;
        // If all port methods are found in the adapter, it likely implements the port
        const allMatch = [...portMethods].every((m) => adapterMethods.has(m));
        if (allMatch && portMethods.size > 0) {
          implementedPorts.add(portName);
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

  /**
   * Collect git state for repo hygiene analysis.
   * Maps git port data into the domain's GitStateSnapshot.
   */
  private async collectGitSnapshot(rootPath: string): Promise<GitStateSnapshot> {
    const git = this.git!;
    const [entries, worktrees, embeddedGitDirs] = await Promise.all([
      git.statusEntries(),
      git.worktreeEntries(),
      git.findEmbeddedRepos(rootPath === '.' ? process.cwd() : rootPath),
    ]);

    return {
      modifiedFiles: entries.filter((e) => e.code === ' M' || e.code === ' D').map((e) => e.path),
      stagedFiles: entries.filter((e) => e.code.startsWith('M') || e.code.startsWith('A') || e.code.startsWith('D')).map((e) => e.path),
      untrackedPaths: entries.filter((e) => e.code === '??').map((e) => e.path),
      worktrees: worktrees.map((w) => ({
        path: w.path,
        branch: w.branch ?? '(detached)',
        commit: w.commit ?? '',
        hasRecentCommits: w.hasRecentCommits,
      })),
      embeddedGitDirs,
    };
  }
}
