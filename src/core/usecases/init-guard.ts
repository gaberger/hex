/**
 * Init Guard
 *
 * Pre-init validation use-case that assesses project size, checks for
 * potential issues, and validates dependency presence before hex init
 * runs. Prevents runaway scans on massive monorepos and surfaces
 * actionable warnings early.
 */

import type { IFileSystemPort } from '../ports/index.js';

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

export const INIT_LIMITS = {
  MAX_FILES: 1_000_000,
  MAX_IGNORE_RULES: 1_000,
  LARGE_PROJECT_FILE_THRESHOLD: 10_000,
  LARGE_PROJECT_SIZE_BYTES: 1_000_000_000, // 1 GB
} as const;

/** Directories that are typically large and should be warned about. */
const KNOWN_LARGE_DIRS = [
  'node_modules/',
  'target/',
  '.git/',
  'dist/',
  'build/',
  '.next/',
  'vendor/',
];

/** Paths hex expects to find (relative to project root). */
const HEX_DEPENDENCY_PATHS = [
  'node_modules/@anthropic-ai',
  'node_modules/typescript',
] as const;

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

export interface ProjectStats {
  estimatedFiles: number;
  estimatedSizeBytes: number;
  isLargeProject: boolean;
}

export interface InitGuardResult {
  canProceed: boolean;
  warnings: string[];
  errors: string[];
  projectStats: ProjectStats;
}

export interface DependencyCheckResult {
  missing: string[];
  warnings: string[];
}

// ---------------------------------------------------------------------------
// Use-case
// ---------------------------------------------------------------------------

export class InitGuard {
  constructor(private readonly fs: IFileSystemPort) {}

  /**
   * Quick shallow scan (maxDepth: 2) to estimate project size and flag
   * potential issues before a full init.
   */
  async assessProject(rootPath: string): Promise<InitGuardResult> {
    const warnings: string[] = [];
    const errors: string[] = [];
    let estimatedFiles = 0;
    let estimatedSizeBytes = 0;

    // Check for known large directories
    for (const dir of KNOWN_LARGE_DIRS) {
      const dirPath = `${rootPath}/${dir.replace(/\/$/, '')}`;
      const exists = await this.fs.exists(dirPath);
      if (exists) {
        warnings.push(
          `Found ${dir} — consider adding to ignore rules to speed up init`,
        );
      }
    }

    // Shallow scan: depth-limited file enumeration
    try {
      for await (const _file of this.fs.streamFiles('**/*', {
        maxDepth: 2,
        ignore: ['node_modules/', '.git/', 'target/', 'vendor/'],
      })) {
        estimatedFiles++;
      }
    } catch {
      errors.push('Unable to scan project directory — check read permissions');
    }

    // Extrapolate: files at depth 2 are a rough lower bound.
    // A typical project has ~5x more files beyond depth 2.
    const extrapolationFactor = 5;
    const totalEstimate = estimatedFiles * extrapolationFactor;
    // Rough average file size estimate: 4 KB
    estimatedSizeBytes = totalEstimate * 4096;

    const isLargeProject =
      totalEstimate > INIT_LIMITS.LARGE_PROJECT_FILE_THRESHOLD ||
      estimatedSizeBytes > INIT_LIMITS.LARGE_PROJECT_SIZE_BYTES;

    if (isLargeProject) {
      warnings.push(
        `Large project detected (~${totalEstimate.toLocaleString()} files). Init may be slow.`,
      );
    }

    if (totalEstimate > INIT_LIMITS.MAX_FILES) {
      errors.push(
        `Project too large for default init (~${totalEstimate.toLocaleString()} estimated files, limit ${INIT_LIMITS.MAX_FILES.toLocaleString()})`,
      );
    }

    const canProceed = errors.length === 0;

    return {
      canProceed,
      warnings,
      errors,
      projectStats: {
        estimatedFiles: totalEstimate,
        estimatedSizeBytes,
        isLargeProject,
      },
    };
  }

  /**
   * Check for presence of key dependency paths that hex needs.
   * Returns missing paths as warnings (deps can be installed later).
   */
  async validateDependencies(
    rootPath: string,
  ): Promise<DependencyCheckResult> {
    const missing: string[] = [];
    const warnings: string[] = [];

    for (const depPath of HEX_DEPENDENCY_PATHS) {
      const fullPath = `${rootPath}/${depPath}`;
      const exists = await this.fs.exists(fullPath);
      if (!exists) {
        missing.push(depPath);
        warnings.push(
          `Missing ${depPath} — run \`npm install\` or \`bun install\` before using hex`,
        );
      }
    }

    return { missing, warnings };
  }
}
