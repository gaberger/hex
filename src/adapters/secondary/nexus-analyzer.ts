/**
 * Nexus Analyzer — IArchAnalysisPort backed by hex-nexus REST API (Rust).
 *
 * Replaces the deleted TypeScript ArchAnalyzer (ADR-034).
 * All analysis is performed by the native Rust tree-sitter engine in hex-nexus.
 */

import type {
  IArchAnalysisPort,
  ImportEdge,
  DeadExport,
  DependencyViolation,
  ArchAnalysisResult,
} from '../../core/ports/index.js';

const DEFAULT_NEXUS_URL = 'http://127.0.0.1:5555';
const TIMEOUT_MS = 30_000;

export class NexusAnalyzer implements IArchAnalysisPort {
  private readonly url: string;

  constructor(nexusUrl?: string) {
    this.url = nexusUrl ?? process.env['HEX_NEXUS_URL'] ?? DEFAULT_NEXUS_URL;
  }

  async buildDependencyGraph(_rootPath: string): Promise<ImportEdge[]> {
    // Not exposed as a separate endpoint — use analyzeArchitecture()
    return [];
  }

  async findDeadExports(rootPath: string): Promise<DeadExport[]> {
    const result = await this.analyzeArchitecture(rootPath);
    return result.deadExports;
  }

  async validateHexBoundaries(rootPath: string): Promise<DependencyViolation[]> {
    const result = await this.analyzeArchitecture(rootPath);
    return result.dependencyViolations;
  }

  async detectCircularDeps(rootPath: string): Promise<string[][]> {
    const result = await this.analyzeArchitecture(rootPath);
    return result.circularDeps;
  }

  async analyzeArchitecture(rootPath: string): Promise<ArchAnalysisResult> {
    const absPath = rootPath.startsWith('/')
      ? rootPath
      : `${process.cwd()}/${rootPath}`.replace(/\/\.$/, '');

    const controller = new AbortController();
    const timeout = setTimeout(() => controller.abort(), TIMEOUT_MS);

    try {
      const res = await fetch(`${this.url}/api/analyze`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ root_path: absPath }),
        signal: controller.signal,
      });

      if (!res.ok) {
        const body = await res.text().catch(() => 'unknown');
        throw new Error(`hex-nexus analysis failed (${res.status}): ${body}`);
      }

      const data = await res.json() as RustResponse;
      return mapResponse(data);
    } finally {
      clearTimeout(timeout);
    }
  }
}

// ── Response mapping (snake_case Rust → camelCase TS) ────

interface RustResponse {
  health_score: number;
  file_count: number;
  edge_count: number;
  violations: Array<{
    edge: { from_file: string; to_file: string; from_layer: string; to_layer: string };
    rule: string;
  }>;
  dead_exports: Array<{ file: string; export_name: string; line: number }>;
  circular_deps: string[][];
  orphan_files: string[];
  unused_ports: string[];
}

function mapResponse(r: RustResponse): ArchAnalysisResult {
  return {
    deadExports: (r.dead_exports ?? []).map((d) => ({
      filePath: d.file,
      exportName: d.export_name,
      kind: 'const' as const,
    })),
    dependencyViolations: (r.violations ?? []).map((v) => ({
      from: v.edge.from_file,
      to: v.edge.to_file,
      fromLayer: v.edge.from_layer as any,
      toLayer: v.edge.to_layer as any,
      rule: v.rule,
    })),
    circularDeps: r.circular_deps ?? [],
    orphanFiles: r.orphan_files ?? [],
    unusedPorts: r.unused_ports ?? [],
    unusedAdapters: [],
    summary: {
      totalFiles: r.file_count ?? 0,
      totalExports: 0,
      deadExportCount: (r.dead_exports ?? []).length,
      violationCount: (r.violations ?? []).length,
      circularCount: (r.circular_deps ?? []).length,
      healthScore: r.health_score ?? 0,
    },
  };
}
