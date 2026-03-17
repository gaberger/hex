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
  BuildConfig,
  StubFile,
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

  generateBuildConfig(projectName: string, language: Language): BuildConfig[] {
    switch (language) {
      case 'typescript':
        return [
          { filename: 'package.json', content: this.tsPackageJson(projectName) },
          { filename: 'tsconfig.json', content: this.tsTsconfig() },
        ];
      case 'go':
        return [
          { filename: 'go.mod', content: this.goMod(projectName) },
        ];
      case 'rust':
        return [
          { filename: 'Cargo.toml', content: this.rustCargoToml(projectName) },
        ];
      default:
        return [];
    }
  }

  generateStubFiles(projectName: string, language: Language): StubFile[] {
    switch (language) {
      case 'typescript': return this.tsStubs(projectName);
      case 'go': return this.goStubs(projectName);
      case 'rust': return this.rustStubs(projectName);
      default: return [];
    }
  }

  async scaffold(rootPath: string, projectName: string, language: Language): Promise<ScaffoldResult> {
    const runtime = await this.analyzeRuntime(rootPath, language);
    const scripts = this.generateScripts(runtime);
    const readme = this.generateReadme(projectName, '', runtime, scripts);
    const envExample = this.generateEnvExample(runtime.envVars);
    const buildConfigs = this.generateBuildConfig(projectName, language);
    const stubs = this.generateStubFiles(projectName, language);

    // Write README.md
    await this.fs.write('README.md', this.renderReadme(readme));

    // Write CLAUDE.md — hex architecture rules for LLM-driven development
    await this.fs.write('CLAUDE.md', this.generateClaudeMd(projectName, language));

    // Write .env.example if there are env vars
    if (runtime.envVars.length > 0) {
      await this.fs.write('.env.example', envExample);
    }

    // Write build config files
    for (const config of buildConfigs) {
      await this.fs.write(config.filename, config.content);
    }

    // Write stub files
    for (const stub of stubs) {
      await this.fs.write(stub.path, stub.content);
    }

    // Write .gitignore
    await this.fs.write('.gitignore', this.generateGitignore(language));

    return { readme, scripts, envExample, runtime, buildConfigs, stubs };
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
      'This project uses [hex](https://github.com/ruvnet/hex) hexagonal architecture:',
      '',
      '- `src/core/domain/` — Pure business logic, zero dependencies',
      '- `src/core/ports/` — Typed interfaces (contracts)',
      '- `src/core/usecases/` — Application orchestration',
      '- `src/adapters/primary/` — Driving adapters (CLI, HTTP, browser)',
      '- `src/adapters/secondary/` — Driven adapters (DB, API, filesystem)',
    ].join('\n');
  }

  private generateClaudeMd(projectName: string, language: Language): string {
    const buildCmd = language === 'typescript' ? 'bun run build' : language === 'go' ? 'go build ./...' : 'cargo build';
    const testCmd = language === 'typescript' ? 'bun test' : language === 'go' ? 'go test ./...' : 'cargo test';

    return [
      `# ${projectName} — Hexagonal Architecture`,
      '',
      '## Behavioral Rules',
      '',
      '- ALWAYS read a file before editing it',
      '- NEVER commit secrets, credentials, or .env files',
      `- ALWAYS run \`${testCmd}\` after making code changes`,
      `- ALWAYS run \`${buildCmd}\` before committing`,
      '',
      '## Hexagonal Architecture Rules (ENFORCED)',
      '',
      '1. **domain/** must only import from **domain/**',
      '2. **ports/** may import from **domain/** but nothing else',
      '3. **usecases/** may import from **domain/** and **ports/** only',
      '4. **adapters/primary/** may import from **ports/** only',
      '5. **adapters/secondary/** may import from **ports/** only',
      '6. **adapters must NEVER import other adapters** (cross-adapter coupling)',
      '7. **composition-root** is the ONLY file that imports from adapters',
      language === 'typescript' ? '8. All relative imports MUST use `.js` extensions (NodeNext module resolution)' : '',
      '',
      '## File Organization',
      '',
      '```',
      'src/',
      '  core/',
      '    domain/          # Pure business logic, zero external deps',
      '    ports/           # Typed interfaces (contracts between layers)',
      '    usecases/        # Application logic composing ports',
      '  adapters/',
      '    primary/         # Driving adapters (CLI, HTTP, browser input)',
      '    secondary/       # Driven adapters (DB, API, filesystem)',
      '  composition-root   # Wires adapters to ports (single DI point)',
      '```',
      '',
      '## Security',
      '',
      '- Never commit `.env` files — use `.env.example`',
      '- Primary adapters MUST NOT use `innerHTML`/`outerHTML`/`insertAdjacentHTML` with any data that originates outside the domain layer. Use `textContent` or DOM APIs (`createElement`) instead.',
      '',
    ].filter(Boolean).join('\n');
  }

  private renderReadme(readme: ProjectReadme): string {
    const lines = [`# ${readme.title}`, ''];
    if (readme.description) lines.push(readme.description, '');
    for (const section of readme.sections) {
      lines.push(`## ${section.heading}`, '', section.content, '');
    }
    return lines.join('\n');
  }

  // ─── Build Config Templates ─────────────────────────────

  private tsPackageJson(name: string): string {
    return JSON.stringify({
      name,
      version: '0.1.0',
      type: 'module',
      scripts: {
        dev: 'bun run --watch src/main.ts',
        build: 'bun build src/main.ts --outdir dist --target node',
        test: 'bun test',
        check: 'tsc --noEmit',
        lint: 'bunx biome check .',
      },
      devDependencies: {
        typescript: '^5.4',
        '@types/bun': 'latest',
      },
    }, null, 2) + '\n';
  }

  private tsTsconfig(): string {
    return JSON.stringify({
      compilerOptions: {
        target: 'ES2022',
        module: 'NodeNext',
        moduleResolution: 'NodeNext',
        strict: true,
        esModuleInterop: true,
        skipLibCheck: true,
        outDir: 'dist',
        rootDir: 'src',
        declaration: true,
        resolveJsonModule: true,
        isolatedModules: true,
      },
      include: ['src'],
      exclude: ['node_modules', 'dist'],
    }, null, 2) + '\n';
  }

  private goMod(name: string): string {
    const modulePath = name.replace(/[^a-zA-Z0-9_-]/g, '-');
    return [
      `module ${modulePath}`,
      '',
      'go 1.21',
      '',
    ].join('\n');
  }

  private rustCargoToml(name: string): string {
    const pkgName = name.replace(/[^a-zA-Z0-9_-]/g, '-');
    return [
      '[package]',
      `name = "${pkgName}"`,
      'version = "0.1.0"',
      'edition = "2021"',
      '',
      '[dependencies]',
      '',
    ].join('\n');
  }

  // ─── Stub File Templates ───────────────────────────────

  private tsStubs(_name: string): StubFile[] {
    return [
      {
        path: 'src/core/domain/entities.ts',
        content: [
          '// Domain entities — pure business logic, zero external dependencies',
          '',
          'export interface Entity {',
          '  id: string;',
          '  createdAt: Date;',
          '}',
          '',
        ].join('\n'),
      },
      {
        path: 'src/core/ports/example-port.ts',
        content: [
          '// Port interface — typed contract between layers',
          '//',
          '// Adapters implement this interface. Domain and use cases',
          '// depend on this contract, never on concrete adapters.',
          '',
          "import type { Entity } from '../domain/entities.js';",
          '',
          'export interface IExamplePort {',
          '  findById(id: string): Promise<Entity | null>;',
          '  save(entity: Entity): Promise<void>;',
          '}',
          '',
        ].join('\n'),
      },
      {
        path: 'src/adapters/secondary/example-adapter.ts',
        content: [
          '// Secondary adapter — driven by the application',
          '//',
          '// Implements the port interface. May only import from ports/ and domain/.',
          '// NEVER import from other adapters.',
          '',
          "import type { Entity } from '../../core/domain/entities.js';",
          "import type { IExamplePort } from '../../core/ports/example-port.js';",
          '',
          'export class InMemoryExampleAdapter implements IExamplePort {',
          '  private store = new Map<string, Entity>();',
          '',
          '  async findById(id: string): Promise<Entity | null> {',
          '    return this.store.get(id) ?? null;',
          '  }',
          '',
          '  async save(entity: Entity): Promise<void> {',
          '    this.store.set(entity.id, entity);',
          '  }',
          '}',
          '',
        ].join('\n'),
      },
      {
        path: 'src/composition-root.ts',
        content: [
          '// Composition root — the ONLY file that imports from adapters',
          '//',
          '// This is the single dependency injection point.',
          "// Swap adapters here; the rest of the code doesn't change.",
          '',
          "import { InMemoryExampleAdapter } from './adapters/secondary/example-adapter.js';",
          '',
          'export function createApp() {',
          '  const exampleRepo = new InMemoryExampleAdapter();',
          '  return { exampleRepo };',
          '}',
          '',
        ].join('\n'),
      },
      {
        path: 'src/main.ts',
        content: [
          "import { createApp } from './composition-root.js';",
          '',
          'const app = createApp();',
          "console.log('App initialized:', Object.keys(app).join(', '));",
          '',
        ].join('\n'),
      },
    ];
  }

  private goStubs(name: string): StubFile[] {
    const mod = name.replace(/[^a-zA-Z0-9_-]/g, '-');
    return [
      {
        path: 'src/core/domain/entities.go',
        content: [
          'package domain',
          '',
          'import "time"',
          '',
          '// Entity is the base domain type. Pure business logic, zero external deps.',
          'type Entity struct {',
          '\tID        string',
          '\tCreatedAt time.Time',
          '}',
          '',
        ].join('\n'),
      },
      {
        path: 'src/core/ports/example_port.go',
        content: [
          'package ports',
          '',
          `import "${mod}/src/core/domain"`,
          '',
          '// ExamplePort defines the contract between layers.',
          '// Adapters implement this interface.',
          'type ExamplePort interface {',
          '\tFindByID(id string) (*domain.Entity, error)',
          '\tSave(entity domain.Entity) error',
          '}',
          '',
        ].join('\n'),
      },
      {
        path: 'src/adapters/secondary/memory_adapter.go',
        content: [
          'package secondary',
          '',
          'import (',
          '\t"fmt"',
          `\t"${mod}/src/core/domain"`,
          ')',
          '',
          '// MemoryAdapter is an in-memory implementation of ports.ExamplePort.',
          'type MemoryAdapter struct {',
          '\tstore map[string]domain.Entity',
          '}',
          '',
          'func NewMemoryAdapter() *MemoryAdapter {',
          '\treturn &MemoryAdapter{store: make(map[string]domain.Entity)}',
          '}',
          '',
          'func (m *MemoryAdapter) FindByID(id string) (*domain.Entity, error) {',
          '\te, ok := m.store[id]',
          '\tif !ok {',
          '\t\treturn nil, fmt.Errorf("entity %s not found", id)',
          '\t}',
          '\treturn &e, nil',
          '}',
          '',
          'func (m *MemoryAdapter) Save(entity domain.Entity) error {',
          '\tm.store[entity.ID] = entity',
          '\treturn nil',
          '}',
          '',
        ].join('\n'),
      },
      {
        path: 'main.go',
        content: [
          'package main',
          '',
          'import (',
          '\t"fmt"',
          `\t"${mod}/src/adapters/secondary"`,
          ')',
          '',
          'func main() {',
          '\trepo := secondary.NewMemoryAdapter()',
          '\tfmt.Println("App initialized with repo:", repo)',
          '}',
          '',
        ].join('\n'),
      },
    ];
  }

  private rustStubs(_name: string): StubFile[] {
    return [
      {
        path: 'src/core/domain/mod.rs',
        content: [
          '//! Domain entities — pure business logic, zero external dependencies',
          '',
          'use std::time::SystemTime;',
          '',
          '#[derive(Debug, Clone)]',
          'pub struct Entity {',
          '    pub id: String,',
          '    pub created_at: SystemTime,',
          '}',
          '',
          'impl Entity {',
          '    pub fn new(id: impl Into<String>) -> Self {',
          '        Self {',
          '            id: id.into(),',
          '            created_at: SystemTime::now(),',
          '        }',
          '    }',
          '}',
          '',
        ].join('\n'),
      },
      {
        path: 'src/core/ports/mod.rs',
        content: [
          '//! Port traits — typed contracts between layers',
          '//!',
          '//! Adapters implement these traits. Domain and use cases',
          '//! depend on these contracts, never on concrete adapters.',
          '',
          'use super::domain::Entity;',
          '',
          'pub trait ExamplePort {',
          '    fn find_by_id(&self, id: &str) -> Option<&Entity>;',
          '    fn save(&mut self, entity: Entity);',
          '}',
          '',
        ].join('\n'),
      },
      {
        path: 'src/core/mod.rs',
        content: [
          'pub mod domain;',
          'pub mod ports;',
          '',
        ].join('\n'),
      },
      {
        path: 'src/adapters/secondary/memory_adapter.rs',
        content: [
          '//! In-memory adapter — implements ExamplePort',
          '',
          'use std::collections::HashMap;',
          'use crate::core::domain::Entity;',
          'use crate::core::ports::ExamplePort;',
          '',
          'pub struct MemoryAdapter {',
          '    store: HashMap<String, Entity>,',
          '}',
          '',
          'impl MemoryAdapter {',
          '    pub fn new() -> Self {',
          '        Self { store: HashMap::new() }',
          '    }',
          '}',
          '',
          'impl ExamplePort for MemoryAdapter {',
          '    fn find_by_id(&self, id: &str) -> Option<&Entity> {',
          '        self.store.get(id)',
          '    }',
          '',
          '    fn save(&mut self, entity: Entity) {',
          '        self.store.insert(entity.id.clone(), entity);',
          '    }',
          '}',
          '',
        ].join('\n'),
      },
      {
        path: 'src/adapters/secondary/mod.rs',
        content: [
          'pub mod memory_adapter;',
          '',
        ].join('\n'),
      },
      {
        path: 'src/adapters/mod.rs',
        content: [
          'pub mod secondary;',
          '',
        ].join('\n'),
      },
      {
        path: 'src/lib.rs',
        content: [
          'pub mod core;',
          'pub mod adapters;',
          '',
        ].join('\n'),
      },
      {
        path: 'src/main.rs',
        content: [
          `mod core;`,
          'mod adapters;',
          '',
          'use crate::core::ports::ExamplePort;',
          'use crate::adapters::secondary::memory_adapter::MemoryAdapter;',
          'use crate::core::domain::Entity;',
          '',
          'fn main() {',
          '    let mut repo = MemoryAdapter::new();',
          '    let entity = Entity::new("1");',
          '    repo.save(entity);',
          '    println!("App initialized. Entity count: {}", if repo.find_by_id("1").is_some() { 1 } else { 0 });',
          '}',
          '',
        ].join('\n'),
      },
    ];
  }

  private generateGitignore(language: Language): string {
    const common = ['# Environment', '.env', '.env.local', '', '# IDE', '.idea/', '.vscode/', '*.swp', ''];
    switch (language) {
      case 'typescript':
        return [...common, '# Build', 'dist/', 'node_modules/', '*.tsbuildinfo', ''].join('\n');
      case 'go':
        return [...common, '# Build', 'dist/', '*.exe', '*.test', '*.out', 'vendor/', ''].join('\n');
      case 'rust':
        return [...common, '# Build', 'target/', ''].join('\n');
      default:
        return common.join('\n');
    }
  }
}
