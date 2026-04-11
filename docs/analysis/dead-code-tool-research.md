# Research: How TypeScript Dead-Code Detection Tools Work

## Source: Direct analysis of tool source code via GitHub API (2026-03-17)

---

## 1. knip (https://knip.dev) — Most Popular TS Dead Code Detector

### Architecture Overview

knip uses the **TypeScript Compiler API** (`ts.createProgram`, `ts.TypeChecker`) as its core analysis engine. It is NOT tree-sitter-based. Key source files:

- `ProjectPrincipal.ts` — Manages TS program creation, entry/project paths, compiler hosts
- `WorkspaceWorker.ts` — Handles per-workspace config, entry file patterns, plugin configs
- `ConfigurationChief.ts` — Resolves configuration, workspace discovery, default patterns
- `typescript/get-imports-and-exports.ts` — Core AST walker for import/export extraction
- `typescript/visitors/dynamic-imports/` — Specialized visitors for dynamic import patterns

### Entry File Concept

knip has two glob arrays in its config:

```json
{
  "entry": ["src/index.ts", "src/cli.ts"],
  "project": ["src/**/*.ts"]
}
```

- **`entry`**: Files whose exports are the "public API" — their exports are NOT reported as unused (unless `isIncludeEntryExports: true`).
- **`project`**: All source files to analyze. Exports in non-entry project files that have no in-project consumers ARE reported as dead.

Default entry patterns (from `ConfigurationChief.ts`):
```typescript
const defaultBaseFilenamePattern = '{index,cli,main}';
// Resolves to:
// entry: ['{index,cli,main}.{js,mjs,cjs,...}!', 'src/{index,cli,main}.{js,mjs,cjs,...}!']
// project: ['**/*.{js,mjs,cjs,...}!']
```

The `!` suffix denotes "production" entry files (used in `--production` mode).

Key config option: `isIncludeEntryExports: false` (default) means entry file exports are assumed to be public API and skipped. When `true`, even entry file exports are checked for consumers.

The `skipExportsAnalysis` set in `ProjectPrincipal` stores paths of config/plugin entry files whose exports should never be reported.

### How knip Handles Dynamic Imports

knip has **7 specialized dynamic import visitors** in `typescript/visitors/dynamic-imports/`:

| Visitor | What it handles |
|---------|----------------|
| `importCall.ts` | `import('specifier')` — the main dynamic import |
| `importType.ts` | `import type { T } from 'specifier'` at type position |
| `jsDocType.ts` | `@type {import('specifier').T}` in JSDoc |
| `requireCall.ts` | `require('specifier')` |
| `resolveCall.ts` | `require.resolve('specifier')` |
| `moduleRegister.ts` | `module.register(...)` |
| `urlConstructor.ts` | `new URL('specifier', import.meta.url)` |

The `importCall.ts` visitor is the most sophisticated. It handles these patterns:

```typescript
// Pattern 1: Property access after import
(await import('./module')).namedExport;

// Pattern 2: Destructuring
const { namedExport } = await import('./module');

// Pattern 3: .then() chain
import('./module').then(m => m.namedExport);
import('./module').then(({ namedExport }) => ...);

// Pattern 4: Promise.all destructuring
const [{ a }, { default: b, c }] = await Promise.all([import('A'), import('B')]);

// Pattern 5: Assigned to variable, then accessed
const mod = await import('./module');
mod.something; // tracked via getAccessedIdentifiers()

// Pattern 6: Element access
(await import('specifier'))['identifier']

// Pattern 7: Side-effects only
import('side-effects'); // marked as SIDE_EFFECTS

// Pattern 8: Opaque (passed to function, can't track)
someFunction(import('./module')); // marked as OPAQUE
```

**Critical insight**: knip resolves the string literal in `import()` using the TS module resolver. If the argument is not a string literal (e.g., `import(variable)`), it cannot resolve it. This is marked as OPAQUE.

For hex's composition-root pattern where adapters are loaded via `await import('./path')`, knip would correctly track these IF the import paths are string literals — which they are in hex.

### How knip Handles package.json "exports" Field

knip does NOT directly parse `package.json` `exports` field to determine entry files. Instead:

1. The **entry file concept** is config-driven (glob patterns in `knip.json`)
2. Default patterns (`{index,cli,main}.{ts,js,...}`) approximate what `package.json` "exports" and "main" would point to
3. Plugin system can add additional entry files based on tool-specific config (e.g., Next.js plugin adds `pages/**/*.ts`)

For workspaces, knip reads `package.json` to discover workspace packages but uses its own `entry`/`project` globs per workspace, not the package's `exports` field.

### JSDoc Tag Handling (Annotation Mechanism)

knip recognizes these JSDoc tags as annotations to control dead-export reporting:

```typescript
// From constants.ts:
export const PUBLIC_TAG = '@public';
export const INTERNAL_TAG = '@internal';
export const BETA_TAG = '@beta';
export const ALIAS_TAG = '@alias';
```

From `util/tag.ts`, the `getShouldIgnoreHandler` function:

```typescript
export const getShouldIgnoreHandler = (isProduction: boolean) => (jsDocTags: Set<string>) =>
  jsDocTags.has(PUBLIC_TAG) ||      // @public → always skip
  jsDocTags.has(BETA_TAG) ||        // @beta → always skip
  jsDocTags.has(ALIAS_TAG) ||       // @alias → always skip
  (isProduction && jsDocTags.has(INTERNAL_TAG)); // @internal → skip only in production
```

**Usage**: Add `/** @public */` above an export and knip will never report it as unused.

knip also supports CLI `--tags` flag for custom filtering:
```bash
knip --tags=+public,-internal  # Include @public, exclude @internal
```

The `getJSDocTags` function in `ast-helpers.ts` uses `ts.getJSDocTags(node)` (TypeScript compiler API) to extract JSDoc tags. It handles:
- Tags on the node itself
- Tags on parent for export specifiers, binding elements
- Tags on parent for enum members, class elements
- Tags on parent for call expressions

### Workspace Configuration

Each workspace gets its own config via the `workspaces` field:

```json
{
  "workspaces": {
    "packages/*": {
      "entry": ["src/index.ts"],
      "project": ["src/**/*.ts"]
    }
  }
}
```

### Barrel Re-exports

knip's import visitor in `get-imports-and-exports.ts` explicitly tracks re-export patterns:

```typescript
// export * from 'specifier' → reExport map
// export * as NS from 'specifier' → reExportNs map
// export { id } from 'specifier' → reExport map
// export { id as alias } from 'specifier' → reExportAs map
// module.exports = require('specifier') → reExport map
```

Barrel files (`index.ts` that re-exports) are tracked as having re-export relationships. The graph explorer then walks these re-export chains to determine if the original export is ultimately consumed.

---

## 2. ts-prune — Simpler, Older Tool

### Architecture

ts-prune uses `ts-morph` (a TypeScript compiler wrapper) rather than raw TS API. Source: `analyzer.ts`.

### False Positive Handling

**Comment directive**: `// ts-prune-ignore-next`

From `constants.ts`:
```typescript
export const ignoreComment = "ts-prune-ignore-next";
```

From `analyzer.ts`, the `mustIgnore` function:
- Gets the symbol's declaration start position
- Finds the node at that position
- Checks `getLeadingCommentRanges()` on that node
- If any leading comment contains `ts-prune-ignore-next`, the export is skipped

**Usage**:
```typescript
// ts-prune-ignore-next
export function intentionallyPublic() { }
```

### Dynamic Import Handling

ts-prune's `handleDynamicImport` is extremely simple:

```typescript
function handleDynamicImport(node: SourceFileReferencingNodes) {
  // a dynamic import always imports all elements, so we can't tell if only some are used
  return ["*"];
}
```

**This means**: Any file that is dynamically imported has ALL its exports marked as "used" (the `"*"` wildcard). This is conservative (no false positives for dynamic imports) but imprecise (no dead export detection within dynamically imported modules).

### Wildcard Import Tracking

ts-prune does track `import * as NS` usage via `trackWildcardUses`:
- `NS.property` → tracks `property` as used
- `NS['property']` → tracks `property` as used
- `const { x } = NS` → tracks `x` as used
- `type T = NS.TypeName` → tracks `TypeName` as used
- Any unrecognized pattern → falls back to `["*"]` (conservative)

### Barrel Re-exports

ts-prune handles `ExportDeclaration` nodes (re-exports) by extracting named exports. But it does not deeply trace re-export chains the way knip does.

### Entry Points

ts-prune does NOT have a first-class "entry point" concept. It analyzes the entire TS project as defined by `tsconfig.json`. All exports that have no in-project importers are reported as unused, regardless of whether they're in entry files.

---

## 3. unimported — Focuses on Unused Files

### Entry Point Configuration

From `config.ts`, unimported uses `.unimportedrc.json`:

```json
{
  "entry": [
    "src/index.ts",
    {
      "file": "src/cli.ts",
      "label": "CLI",
      "aliases": { "@/": ["src/"] },
      "extensions": [".ts", ".tsx"],
      "ignore": ["**/*.test.ts"]
    }
  ],
  "ignorePatterns": ["**/*.test.ts", "**/__mocks__/**"],
  "ignoreUnresolved": ["virtual:*"],
  "ignoreUnimported": ["src/types/**"],
  "ignoreUnused": ["lodash"]
}
```

Entry files can be strings (simple) or objects (with per-entry aliases, extensions, ignore patterns).

### Presets

unimported has a **preset system** that auto-detects frameworks:
- Checks `package.json` dependencies to detect Next.js, Gatsby, Meteor, etc.
- Each preset provides default entry files and ignore patterns

### Dynamic Import Handling

unimported traverses the import graph from entry files. It follows:
- Static `import` / `require` statements
- `import()` calls (resolves the string literal argument)

Files not reachable from any entry file are reported as "unimported."

### Ignore Mechanisms

| Config key | Purpose |
|-----------|---------|
| `ignorePatterns` | Glob patterns for files to skip entirely |
| `ignoreUnresolved` | Module specifiers that can't be resolved (virtual modules, etc.) |
| `ignoreUnimported` | Files that are intentionally not imported (e.g., type-only files) |
| `ignoreUnused` | Package dependencies that appear unused but are needed |

### Barrel Re-exports

unimported treats barrel files as regular files in the import graph. If `index.ts` re-exports from `foo.ts`, and `index.ts` is reachable from an entry file, then `foo.ts` is also reachable. It does NOT analyze individual export names — it only checks file reachability.

---

## 4. ESLint plugin no-unused-exports

### Overview

This is typically `eslint-plugin-import` with the `no-unused-modules` rule, or the newer `eslint-plugin-import-x`.

### Configuration

```json
{
  "rules": {
    "import/no-unused-modules": [
      "error",
      {
        "unusedExports": true,
        "missingExports": false,
        "src": ["src/**/*.ts"],
        "ignoreExports": [
          "src/index.ts",
          "src/cli.ts",
          "**/*.test.ts",
          "**/*.stories.ts"
        ]
      }
    ]
  }
}
```

### Ignore Patterns

- `ignoreExports`: Array of glob patterns for files whose exports should not be checked
- No per-export annotation mechanism (file-level only)
- Standard ESLint `// eslint-disable-next-line import/no-unused-modules` for individual lines

### Dynamic Import Handling

The ESLint plugin uses its own resolver system. `import()` calls with string literals are resolved. Dynamic/computed specifiers are not.

### Barrel Re-exports

The plugin follows re-export chains (`export { x } from './foo'`) to determine if the original export is consumed downstream.

---

## 5. Tree-sitter and JSDoc/Comment Extraction

### Can tree-sitter extract JSDoc tags?

**Yes, but with caveats.**

The TypeScript tree-sitter grammar (`tree-sitter-typescript`) parses comments as `comment` nodes. These are sibling nodes to the declarations they precede, NOT child nodes of the declaration.

**What tree-sitter provides**:
- `comment` node type for `//` and `/* */` comments
- The comment text is available via `node.text`
- Comments appear as siblings in the tree, preceding the declaration

**What tree-sitter does NOT provide**:
- No built-in JSDoc parsing (no `@param`, `@returns`, `@public` extraction)
- No `ts.getJSDocTags()` equivalent — you must regex-match the comment text yourself
- No automatic association between a comment and its "attached" declaration

**Practical approach for hex** (from existing `treesitter-adapter.ts` line 256-277):

The hex tree-sitter adapter already skips `comment` nodes when looking for declarations inside export statements:
```typescript
const decl = node.namedChildren.find((c) => c != null && c.type !== 'comment');
```

To extract JSDoc annotations, the approach would be:

```typescript
// For each export_statement node:
// 1. Get the previous sibling
const prevSibling = exportNode.previousNamedSibling;
// 2. Check if it's a comment
if (prevSibling?.type === 'comment') {
  const text = prevSibling.text;
  // 3. Regex match for annotations
  if (/@hex:(public|dynamic|entry)/.test(text)) {
    // Mark this export as intentionally public
  }
  if (/@public/.test(text)) {
    // knip-compatible: treat as public API
  }
}
```

**Tree-sitter query (S-expression)**:
```scheme
;; Match comment immediately before export_statement
(program
  (comment) @doc
  (export_statement) @export
  (#match? @doc "@hex:(public|dynamic|entry)")
)
```

**Limitation**: Tree-sitter queries match patterns within the tree, but "immediately preceding sibling" is hard to express in a single query pattern. The above query matches any comment followed by any export in the program, not necessarily adjacent. The practical implementation should use the programmatic API (previousNamedSibling) rather than a declarative query.

---

## Comparison Matrix

| Feature | knip | ts-prune | unimported | ESLint plugin |
|---------|------|----------|------------|---------------|
| **Analysis engine** | TS Compiler API | ts-morph (TS wrapper) | Custom traverser | ESLint + resolver |
| **Granularity** | Per-export | Per-export | Per-file | Per-export |
| **Entry file concept** | Yes (`entry` globs) | No | Yes (`entry` config) | Yes (`ignoreExports` globs) |
| **Dynamic import tracking** | Detailed (7 visitors, pattern-specific) | Conservative (`*` = all used) | File-level reachability | String literal resolution |
| **Per-export annotation** | `/** @public */` JSDoc tag | `// ts-prune-ignore-next` | None | `// eslint-disable-next-line` |
| **Config-level ignore** | `ignore`, `ignoreExportsUsedInFile` | None | `ignoreUnimported`, `ignorePatterns` | `ignoreExports` globs |
| **Barrel re-export tracking** | Deep chain walking | Shallow | File-level only | Chain walking |
| **package.json exports** | Not directly used for entry detection | No | No | No |
| **Workspace support** | Yes (per-workspace config) | No | No | Via ESLint overrides |
| **Plugin system** | Yes (60+ plugins for tools) | No | Presets (framework detection) | Via ESLint ecosystem |

---

## Key Takeaways for hex

### 1. Entry File Pattern (from knip)
knip's `entry` + `isIncludeEntryExports` is the gold standard. hex should adopt:
- Entry files = files whose exports are public API (never flagged as dead)
- Default entry patterns: `{index,cli,main,composition-root}.ts`
- Config override possible but layer-based convention is better for hex

### 2. Dynamic Import Handling (from knip)
knip proves that **string-literal dynamic imports can be fully tracked**. hex's composition root uses `await import('./adapter-path')` with literal paths, so these CAN be resolved. The approach:
- Parse `import()` call expressions
- Extract the string literal argument
- Resolve to a file path
- Mark all exports of that file as "consumed by composition root"

For tree-sitter-based analysis (hex's approach), this means:
1. Find `call_expression` nodes where the function is `import`
2. Extract the string argument
3. Add the resolved file to the import graph

### 3. JSDoc Annotation (from knip + ts-prune)
Two patterns exist in the ecosystem:
- **knip**: `/** @public */` — uses standard TSDoc tags, extracted via TS compiler API
- **ts-prune**: `// ts-prune-ignore-next` — custom comment directive, extracted via leading comment ranges

For hex (tree-sitter based), the recommendation is:
- Use `/** @public */` for ecosystem compatibility with knip
- Additionally support `/** @hex:public */` for hex-specific semantics
- Extract via `previousNamedSibling` check on export nodes in tree-sitter AST

### 4. Conservative vs Precise Dynamic Import Analysis
- **ts-prune**: `import() → all exports used` (conservative, no false positives)
- **knip**: `import() → tracks which specific exports are accessed` (precise but complex)

For hex, the **conservative approach is sufficient**: if a file is dynamically imported in `composition-root.ts`, mark ALL its exports as consumed. This is correct because composition roots wire entire adapters, not individual exports.

### 5. Barrel/Re-export Handling
All tools recognize that barrel files (`index.ts` with `export * from`) are not dead code. hex's existing `hasReExports()` heuristic (>50% of exports match import names) approximates this. A better approach:
- Track re-export chains explicitly (like knip)
- If a file only contains re-exports, it is a barrel — skip dead export analysis on it
- Check if the barrel's consumers are reachable instead

### 6. Tree-sitter Cannot Replace TS Compiler for Dead Code Analysis
All mature tools (knip, ts-prune) use the TypeScript compiler API because:
- Module resolution requires `tsconfig.json` path mappings
- Type-level imports need type checker info
- JSDoc tag extraction uses `ts.getJSDocTags()`

hex uses tree-sitter for fast AST summaries (L0-L3), which is the right tool for summarization. For dead-export analysis, hex should either:
- Use tree-sitter for export discovery + a simpler import graph (current approach)
- Or shell out to the TS compiler for precision (heavier but more accurate)

The current tree-sitter approach is viable IF:
- Import paths are resolved via simple string matching (no path aliases without a resolver)
- Dynamic imports use string literals
- JSDoc tags are extracted via regex on comment text (not TS API)
