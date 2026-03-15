/**
 * Scaffold & Runtime Port
 *
 * Ensures every generated project is immediately runnable.
 * Produces README.md, start scripts, and validates that the
 * project can actually launch — not just compile.
 */

import type { Language } from '../domain/value-objects.js';

// ─── Types ───────────────────────────────────────────────

export type RuntimeTarget = 'browser' | 'node' | 'bun' | 'deno' | 'binary';
export type PackageManager = 'npm' | 'bun' | 'pnpm' | 'yarn';

export interface RuntimeRequirements {
  targets: RuntimeTarget[];
  packageManager: PackageManager;
  devServer?: { tool: string; command: string; port: number };
  buildStep?: { tool: string; command: string; outputDir: string };
  envVars: EnvVarSpec[];
  systemDeps: string[];       // e.g., "git", "docker", "protoc"
  minNodeVersion?: string;
  minBunVersion?: string;
}

export interface EnvVarSpec {
  name: string;
  description: string;
  required: boolean;
  example: string;
  secret: boolean;            // if true, goes in .env.example not README
}

export interface StartScript {
  name: string;               // e.g., "dev", "build", "test", "start"
  command: string;             // e.g., "bun run dev"
  description: string;
  phase: 'setup' | 'dev' | 'test' | 'build' | 'deploy';
}

export interface ReadmeSection {
  heading: string;
  content: string;
}

export interface ProjectReadme {
  title: string;
  description: string;
  sections: ReadmeSection[];
}

export interface ScaffoldResult {
  readme: ProjectReadme;
  scripts: StartScript[];
  envExample: string;         // .env.example content
  runtime: RuntimeRequirements;
}

// ─── Output Port (Secondary / Driven) ────────────────────

export interface IScaffoldPort {
  /** Analyze a project and determine its runtime requirements */
  analyzeRuntime(rootPath: string, language: Language): Promise<RuntimeRequirements>;

  /** Generate start scripts for package.json based on runtime analysis */
  generateScripts(runtime: RuntimeRequirements): StartScript[];

  /** Generate a README.md with setup, run, test, and deploy instructions */
  generateReadme(
    projectName: string,
    description: string,
    runtime: RuntimeRequirements,
    scripts: StartScript[],
  ): ProjectReadme;

  /** Generate .env.example from env var specs */
  generateEnvExample(envVars: EnvVarSpec[]): string;

  /** Full scaffold: analyze → scripts → readme → env → write all files */
  scaffold(rootPath: string, projectName: string, language: Language): Promise<ScaffoldResult>;

  /** Validate that a project can actually start (run dev command, check for errors) */
  validateRunnable(rootPath: string, scripts: StartScript[]): Promise<ValidationResult>;
}

export interface ValidationResult {
  runnable: boolean;
  testedScripts: Array<{
    script: StartScript;
    success: boolean;
    error?: string;
    duration: number;
  }>;
}
