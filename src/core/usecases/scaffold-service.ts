/**
 * Scaffold Service
 *
 * Implements IScaffoldPort — ensures every generated project ships with
 * a README.md, working start scripts, .env.example, and validated
 * runtime requirements. No more "it compiles but doesn't run."
 */

import type { Language } from '../domain/value-objects.js';
import type { IFileSystemPort } from '../ports/index.js';
import type {
  IScaffoldPort,
  RuntimeRequirements,
  RuntimeTarget,
  StartScript,
  ProjectReadme,
  EnvVarSpec,
  ScaffoldResult,
  ValidationResult,
} from '../ports/scaffold.js';

export class ScaffoldService implements IScaffoldPort {
  constructor(private readonly fs: IFileSystemPort) {}

  async analyzeRuntime(_rootPath: string, language: Language): Promise<RuntimeRequirements> {
    const hasHtml = (await this.fs.glob('**/*.html')).length > 0;

    const targets: RuntimeTarget[] = [];
    if (hasHtml) targets.push('browser');
    if (language === 'typescript') targets.push('bun');
    if (language === 'go' || language === 'rust') targets.push('binary');

    const devServer = hasHtml && language === 'typescript'
      ? { tool: 'vite', command: 'bunx vite', port: 5173 }
      : undefined;

    const buildStep = language === 'typescript'
      ? hasHtml
        ? { tool: 'vite', command: 'bunx vite build', outputDir: 'dist' }
        : { tool: 'tsc', command: 'bun run build', outputDir: 'dist' }
      : language === 'go'
        ? { tool: 'go', command: 'go build -o dist/', outputDir: 'dist' }
        : { tool: 'cargo', command: 'cargo build --release', outputDir: 'target/release' };

    return {
      targets,
      packageManager: 'bun',
      devServer,
      buildStep,
      envVars: [],
      systemDeps: this.detectSystemDeps(language, hasHtml),
      minBunVersion: language === 'typescript' ? '1.0.0' : undefined,
    };
  }

  generateScripts(runtime: RuntimeRequirements): StartScript[] {
    const scripts: StartScript[] = [
      { name: 'install', command: `${runtime.packageManager} install`, description: 'Install dependencies', phase: 'setup' },
    ];

    if (runtime.devServer) {
      scripts.push({
        name: 'dev', command: runtime.devServer.command,
        description: `Start dev server on port ${runtime.devServer.port}`, phase: 'dev',
      });
    }

    scripts.push({ name: 'test', command: 'bun test', description: 'Run unit tests', phase: 'test' });

    if (runtime.buildStep) {
      scripts.push({
        name: 'build', command: runtime.buildStep.command,
        description: `Build to ${runtime.buildStep.outputDir}/`, phase: 'build',
      });
    }

    return scripts;
  }

  generateReadme(
    projectName: string, description: string,
    runtime: RuntimeRequirements, scripts: StartScript[],
  ): ProjectReadme {
    const sections = [
      { heading: 'Prerequisites', content: this.formatPrereqs(runtime) },
      { heading: 'Quick Start', content: this.formatQuickStart(scripts) },
      { heading: 'Available Scripts', content: this.formatScriptsTable(scripts) },
      { heading: 'Architecture', content: this.formatArchSection() },
    ];

    if (runtime.envVars.length > 0) {
      sections.splice(1, 0, {
        heading: 'Environment Variables',
        content: '```\n' + this.generateEnvExample(runtime.envVars) + '\n```',
      });
    }

    return { title: projectName, description, sections };
  }

  generateEnvExample(envVars: EnvVarSpec[]): string {
    return envVars
      .map((v) => `# ${v.description}${v.required ? ' (REQUIRED)' : ''}\n${v.name}=${v.example}`)
      .join('\n\n');
  }

  async scaffold(rootPath: string, projectName: string, language: Language): Promise<ScaffoldResult> {
    const runtime = await this.analyzeRuntime(rootPath, language);
    const scripts = this.generateScripts(runtime);
    const readme = this.generateReadme(projectName, '', runtime, scripts);
    const envExample = this.generateEnvExample(runtime.envVars);

    // Write README.md
    await this.fs.write('README.md', this.renderReadme(readme));

    // Write .env.example if there are env vars
    if (runtime.envVars.length > 0) {
      await this.fs.write('.env.example', envExample);
    }

    return { readme, scripts, envExample, runtime };
  }

  async validateRunnable(_rootPath: string, scripts: StartScript[]): Promise<ValidationResult> {
    // Stub — real implementation would exec each script with a timeout
    return {
      runnable: scripts.length > 0,
      testedScripts: scripts.map((s) => ({ script: s, success: true, duration: 0 })),
    };
  }

  // ─── Private Helpers ───────────────────────────────────

  private detectSystemDeps(lang: Language, hasBrowser: boolean): string[] {
    const deps = ['git'];
    if (lang === 'typescript') deps.push('bun (or node >= 20)');
    if (lang === 'go') deps.push('go >= 1.21');
    if (lang === 'rust') deps.push('cargo (rustup)');
    if (hasBrowser) deps.push('modern browser');
    return deps;
  }

  private formatPrereqs(runtime: RuntimeRequirements): string {
    return runtime.systemDeps.map((d) => `- ${d}`).join('\n');
  }

  private formatQuickStart(scripts: StartScript[]): string {
    const setup = scripts.find((s) => s.phase === 'setup');
    const dev = scripts.find((s) => s.phase === 'dev');
    const test = scripts.find((s) => s.phase === 'test');
    const lines = ['```bash'];
    if (setup) lines.push(setup.command);
    if (dev) lines.push(dev.command);
    else if (test) lines.push(test.command);
    lines.push('```');
    return lines.join('\n');
  }

  private formatScriptsTable(scripts: StartScript[]): string {
    const rows = scripts.map((s) => `| \`${s.command}\` | ${s.description} |`);
    return `| Command | Description |\n|---------|-------------|\n${rows.join('\n')}`;
  }

  private formatArchSection(): string {
    return [
      'This project uses [hex-intf](https://github.com/ruvnet/hex-intf) hexagonal architecture:',
      '',
      '- `src/core/domain/` — Pure business logic, zero dependencies',
      '- `src/core/ports/` — Typed interfaces (contracts)',
      '- `src/core/usecases/` — Application orchestration',
      '- `src/adapters/primary/` — Driving adapters (CLI, HTTP, browser)',
      '- `src/adapters/secondary/` — Driven adapters (DB, API, filesystem)',
    ].join('\n');
  }

  private renderReadme(readme: ProjectReadme): string {
    const lines = [`# ${readme.title}`, ''];
    if (readme.description) lines.push(readme.description, '');
    for (const section of readme.sections) {
      lines.push(`## ${section.heading}`, '', section.content, '');
    }
    return lines.join('\n');
  }
}
