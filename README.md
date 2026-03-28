# Hex Project Analysis Engine

The Hex Project Analysis Engine provides language-agnostic analysis capabilities for software projects, enabling deep code understanding through pluggable analyzers. This component implements the core analysis infrastructure that powers features like dependency mapping, structural analysis, and cross-language project insights.

This engine was motivated by [ADR-001: Analysis Engine Architecture](docs/adrs/001-analysis-engine-architecture.md) and spans the **domain** and **ports** layers of the hex architecture.

## Architecture

The Analysis Engine follows a pluggable architecture where different language analyzers can be registered and used interchangeably through a common interface.

```
┌─────────────────────────────────────────────────────────────┐
│                    Hex Project Analysis Engine               │
├─────────────────────────────────────────────────────────────┤
│  Domain Layer                                               │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  Analysis Engine                                         ││
│  │  ┌─────────────────────────────────────────────────────┐││
│  │  │  Port Interfaces (IAnalysisPort, IProjectAnalyzer)   │││
│  │  └─────────────────────────────────────────────────────┘││
│  └─────────────────────────────────────────────────────────┘│
│                                                             │
│  Adapters Layer                                             │
│  ┌─────────────────────────────────────────────────────────┐│
│  │  Language-Specific Adapters                              ││
│  │  ┌─────────────────────────────────────────────────────┐││
│  │  │  TreeSitterAdapter (TypeScript/JavaScript)           │││
│  │  │  RustAnalyzerAdapter (Rust)                          │││
│  │  │  PythonAnalyzerAdapter (Python)                      │││
│  │  └─────────────────────────────────────────────────────┘││
│  └─────────────────────────────────────────────────────────┘│
└─────────────────────────────────────────────────────────────┘
```

**Dependencies:**
- `ports/IAnalysisPort.ts` ← contract
- `adapters/secondary/` ← implementations
  - `TreeSitterAdapter.ts` ← implements IAnalysisPort
  - `RustAnalyzerAdapter.ts` ← implements IAnalysisPort
  - `PythonAnalyzerAdapter.ts` ← implements IAnalysisPort
- `usecases/` ← consumers
  - `AnalyzeProject.ts` ← depends on IAnalysisPort

**Tier:** 2 (Core Business Logic)  
**Layers:** Domain, Ports

## Quick Start

### Prerequisites
- Node.js 18+ or compatible runtime
- Rust 1.60+ (for Rust analyzer)
- Python 3.8+ (for Python analyzer)
- Tree-sitter CLI (for language parsing)

### Installation
```bash
npm install @hex-project/analysis-engine
```

### Running the Engine

#### Development Mode
```bash
# Clone and setup
git clone https://github.com/hex-project/analysis-engine.git
cd analysis-engine
npm install

# Start development server
npm run dev

# Run tests
npm test
```

#### Production Usage
```bash
# Build for production
npm run build

# Run analysis on a project
node dist/cli.js analyze /path/to/project
```

### Environment Variables
```bash
# Configure analysis settings
export HEX_ANALYZER_TIMEOUT=30000
export HEX_MAX_CONCURRENT_ANALYZERS=4
export HEX_LOG_LEVEL=INFO
```

## API Reference

### IAnalysisPort Interface

The core contract for all analysis adapters.

```typescript
// File: ports/IAnalysisPort.ts
export interface IAnalysisPort {
  /**
   * Analyzes a single file and returns its AST and semantic information.
   * @param filePath - Path to the file to analyze
   * @param content - File content (optional, for in-memory analysis)
   * @returns AnalysisResult containing AST, symbols, and diagnostics
   */
  analyzeFile(filePath: string, content?: string): Promise<AnalysisResult>;

  /**
   * Analyzes an entire project directory.
   * @param projectRoot - Root directory of the project
   * @returns ProjectAnalysis containing all files and their analysis
   */
  analyzeProject(projectRoot: string): Promise<ProjectAnalysis>;

  /**
   * Returns supported file extensions for this analyzer.
   * @returns Array of supported extensions (e.g., ['.ts', '.js'])
   */
  getSupportedExtensions(): string[];

  /**
   * Validates if a file can be analyzed by this adapter.
   * @param filePath - Path to validate
   * @returns True if the file is supported
   */
  canAnalyze(filePath: string): boolean;
}
```

### Usage Example
```typescript
import { AnalysisEngine } from './engine/AnalysisEngine';
import { TreeSitterAdapter } from './adapters/secondary/TreeSitterAdapter';

// Create engine and register adapter
const engine = new AnalysisEngine();
engine.registerAdapter(new TreeSitterAdapter());

// Analyze a file
const result = await engine.analyzeFile('/path/to/file.ts');
console.log(result.ast);
console.log(result.symbols);
```

### ProjectAnalysis Structure
```typescript
interface ProjectAnalysis {
  files: AnalysisResult[];      // All analyzed files
  symbols: Symbol[];            // All symbols in project
  dependencies: DependencyGraph; // Dependency relationships
  diagnostics: Diagnostic[];     // Analysis diagnostics
}
```

## Development Guide

### Adding a New Analyzer Adapter

1. **Create the adapter class** implementing `IAnalysisPort`
2. **Add to adapters/secondary/** directory
3. **Register in AnalysisEngine** constructor
4. **Write comprehensive tests** covering all interface methods

```typescript
// Example: NewLanguageAdapter.ts
import { IAnalysisPort } from '../ports/IAnalysisPort';

export class NewLanguageAdapter implements IAnalysisPort {
  async analyzeFile(filePath: string, content?: string): Promise<AnalysisResult> {
    // Implementation here
  }

  // Implement other required methods
}
```

### Testing Conventions

- **London-school testing**: Focus on behavior, use mocks for external dependencies
- **Deps pattern**: All dependencies injected via constructor
- **Test structure**:
  ```typescript
  describe('NewLanguageAdapter', () => {
    let adapter: NewLanguageAdapter;
    let mockParser: MockParser;

    beforeEach(() => {
      mockParser = new MockParser();
      adapter = new NewLanguageAdapter(mockParser);
    });

    it('should analyze valid files correctly', async () => {
      // Test implementation
    });
  });
  ```

### Common Pitfalls

1. **Hex boundary violations**: Adapters should not import from usecases or domain directly
2. **Synchronous operations**: All analysis should be async to handle large projects
3. **Memory leaks**: Large ASTs should be processed in streams when possible
4. **Error handling**: Always return structured errors, never throw unexpectedly

### Architecture Validation

Run the hex architecture validator to ensure compliance:

```bash
npx hex analyze
```

This checks:
- No circular dependencies
- Layer violations
- Missing interface implementations
- Test coverage thresholds

## Related

- [ADR-001: Analysis Engine Architecture](docs/adrs/001-analysis-engine-architecture.md)
- [ADR-002: Language Adapter Interface Design](docs/adrs/002-language-adapter-interface-design.md)
- [IAnalysisPort Interface](ports/IAnalysisPort.ts)
- [Workplan: Analysis Engine Phase 1](docs/workplans/analysis-engine-phase1.md)