import { describe, it, expect } from 'bun:test';
import { resolveImportPath, normalizePath, rustModuleCandidates } from '../../src/core/usecases/path-normalizer.js';
import { classifyLayer, classifySpecialFile } from '../../src/core/usecases/layer-classifier.js';

// ─── Go Export Detection Logic ─────────────────────────────
// Go exports are determined by capitalization of the first letter.
// This helper mirrors the logic used inside TreeSitterAdapter for Go files.

function isCapitalized(name: string): boolean {
  if (name.length === 0) return false;
  const first = name.charAt(0);
  return first === first.toUpperCase() && first !== first.toLowerCase();
}

// ─── Tests ─────────────────────────────────────────────────

describe('detectLanguage (via normalizePath identity)', () => {
  // detectLang is private, but we can verify its behavior indirectly:
  // normalizePath preserves .go and .rs extensions, converts .js → .ts

  it('.go files are recognized as Go (extension preserved)', () => {
    expect(normalizePath('src/main.go')).toBe('src/main.go');
  });

  it('.rs files are recognized as Rust (extension preserved)', () => {
    expect(normalizePath('src/lib.rs')).toBe('src/lib.rs');
  });

  it('.ts files are recognized as TypeScript (extension preserved)', () => {
    expect(normalizePath('src/cli.ts')).toBe('src/cli.ts');
  });

  it('.js files are converted to .ts (TypeScript detection)', () => {
    expect(normalizePath('src/cli.js')).toBe('src/cli.ts');
  });
});

describe('Go export detection — isCapitalized helper', () => {
  it('Handler is exported (capitalized)', () => {
    expect(isCapitalized('Handler')).toBe(true);
  });

  it('handler is not exported (lowercase)', () => {
    expect(isCapitalized('handler')).toBe(false);
  });

  it('HTTPServer is exported (capitalized acronym)', () => {
    expect(isCapitalized('HTTPServer')).toBe(true);
  });

  it('internal is not exported (lowercase)', () => {
    expect(isCapitalized('internal')).toBe(false);
  });

  it('empty string is not exported', () => {
    expect(isCapitalized('')).toBe(false);
  });

  it('underscore-prefixed is not exported', () => {
    expect(isCapitalized('_private')).toBe(false);
  });
});

describe('Path normalizer — Go imports', () => {
  it('resolves relative Go import from nested adapter', () => {
    // dirname('src/adapters/primary/http.go') = 'src/adapters/primary'
    // join('src/adapters/primary', '../ports') = 'src/adapters/ports'
    expect(resolveImportPath('src/adapters/primary/http.go', '../ports')).toBe(
      'src/adapters/ports',
    );
  });

  it('resolves relative Go import going up two levels', () => {
    // dirname('src/adapters/primary/http.go') = 'src/adapters/primary'
    // join('src/adapters/primary', '../../ports') = 'src/ports'
    expect(resolveImportPath('src/adapters/primary/http.go', '../../ports')).toBe('src/ports');
  });

  it('keeps external/stdlib Go imports as-is', () => {
    expect(resolveImportPath('src/main.go', 'net/http')).toBe('net/http');
  });

  it('keeps third-party Go module imports as-is', () => {
    expect(resolveImportPath('src/main.go', 'github.com/gin-gonic/gin')).toBe(
      'github.com/gin-gonic/gin',
    );
  });
});

describe('Path normalizer — Rust imports', () => {
  it('resolves crate::core::ports to src/core/ports', () => {
    expect(resolveImportPath('src/main.rs', 'crate::core::ports')).toBe('src/core/ports');
  });

  it('resolves crate::core::domain to src/core/domain', () => {
    expect(resolveImportPath('src/adapters/primary/http.rs', 'crate::core::domain')).toBe(
      'src/core/domain',
    );
  });

  it('resolves crate:: with deeply nested path — strips uppercase item name', () => {
    expect(
      resolveImportPath('src/adapters/secondary/store.rs', 'crate::core::domain::entities::User'),
    ).toBe('src/core/domain/entities');
  });

  it('strips uppercase FooPort from crate::core::ports::FooPort', () => {
    expect(
      resolveImportPath('src/main.rs', 'crate::core::ports::FooPort'),
    ).toBe('src/core/ports');
  });

  it('keeps lowercase last segment (http_adapter is a module, not an item)', () => {
    expect(
      resolveImportPath('src/main.rs', 'crate::adapters::primary::http_adapter'),
    ).toBe('src/adapters/primary/http_adapter');
  });

  it('keeps external crate imports as-is', () => {
    expect(resolveImportPath('src/main.rs', 'serde')).toBe('serde');
  });

  it('handles super:: imports', () => {
    expect(resolveImportPath('src/adapters/primary/http.rs', 'super::foo')).toBe('super/foo');
  });
});

describe('Path normalizer — Rust normalizePath', () => {
  it('preserves .rs extension', () => {
    expect(normalizePath('src/adapters/secondary/store.rs')).toBe(
      'src/adapters/secondary/store.rs',
    );
  });

  it('strips leading ./ from .rs paths', () => {
    expect(normalizePath('./src/lib.rs')).toBe('src/lib.rs');
  });
});

describe('Path normalizer — Go normalizePath', () => {
  it('preserves .go extension', () => {
    expect(normalizePath('src/core/domain/model.go')).toBe('src/core/domain/model.go');
  });

  it('strips leading ./ from .go paths', () => {
    expect(normalizePath('./src/main.go')).toBe('src/main.go');
  });
});

describe('Path normalizer — TypeScript backward compatibility', () => {
  it('resolves relative .js import to .ts', () => {
    expect(
      resolveImportPath('src/adapters/secondary/git.ts', '../../core/ports/index.js'),
    ).toBe('src/core/ports/index.ts');
  });

  it('normalizePath converts .js to .ts', () => {
    expect(normalizePath('./src/cli.js')).toBe('src/cli.ts');
  });

  it('normalizePath converts .jsx to .tsx', () => {
    expect(normalizePath('./src/App.jsx')).toBe('src/App.tsx');
  });

  it('normalizePath preserves .ts extension', () => {
    expect(normalizePath('src/index.ts')).toBe('src/index.ts');
  });
});

describe('Path normalizer — Rust self:: imports', () => {
  it('resolves self:: import relative to current file directory', () => {
    expect(resolveImportPath('src/adapters/primary/http.rs', 'self::handler')).toBe(
      'src/adapters/primary/handler',
    );
  });

  it('resolves self:: with nested path', () => {
    expect(resolveImportPath('src/core/domain/mod.rs', 'self::entities::User')).toBe(
      'src/core/domain/entities/User',
    );
  });
});

describe('Path normalizer — Go cmd/ entry points', () => {
  it('Go cmd/ paths are kept as-is for non-relative imports', () => {
    expect(resolveImportPath('cmd/server/main.go', 'net/http')).toBe('net/http');
  });

  it('Go cmd/ relative import resolves correctly', () => {
    expect(resolveImportPath('cmd/server/main.go', '../../internal/ports')).toBe('internal/ports');
  });
});

describe('Layer classifier — Go conventions', () => {
  it('classifies cmd/ as primary adapter', () => {
    expect(classifyLayer('cmd/server/main.go')).toBe('adapters/primary');
  });

  it('classifies internal/domain/ as domain', () => {
    expect(classifyLayer('internal/domain/model.go')).toBe('domain');
  });

  it('classifies pkg/ as ports', () => {
    expect(classifyLayer('pkg/api/handler.go')).toBe('ports');
  });

  it('classifies internal/ catch-all as usecases', () => {
    expect(classifyLayer('internal/service/worker.go')).toBe('usecases');
  });
});

describe('Path normalizer — Rust module candidates', () => {
  it('returns both .rs and /mod.rs candidates', () => {
    expect(rustModuleCandidates('src/core/domain')).toEqual([
      'src/core/domain.rs',
      'src/core/domain/mod.rs',
    ]);
  });

  it('works for deeply nested paths', () => {
    expect(rustModuleCandidates('src/adapters/primary/http_adapter')).toEqual([
      'src/adapters/primary/http_adapter.rs',
      'src/adapters/primary/http_adapter/mod.rs',
    ]);
  });
});

describe('Path normalizer — Rust self:: from mod declaration', () => {
  it('self::foo resolves relative to current module directory', () => {
    expect(resolveImportPath('src/lib.rs', 'self::foo')).toBe('src/foo');
  });

  it('self::foo resolves from nested module file', () => {
    expect(resolveImportPath('src/adapters/mod.rs', 'self::primary')).toBe(
      'src/adapters/primary',
    );
  });
});

describe('Layer classifier — Rust conventions', () => {
  it('classifies src/bin/ as primary adapter', () => {
    expect(classifyLayer('src/bin/server.rs')).toBe('adapters/primary');
  });

  it('classifies src/routes/ as primary adapter', () => {
    expect(classifyLayer('src/routes/push.rs')).toBe('adapters/primary');
  });

  it('classifies src/handlers/ as primary adapter', () => {
    expect(classifyLayer('src/handlers/webhook.rs')).toBe('adapters/primary');
  });

  it('classifies src/middleware/ as primary adapter', () => {
    expect(classifyLayer('src/middleware/auth.rs')).toBe('adapters/primary');
  });

  it('classifies src/embed.rs as infrastructure', () => {
    expect(classifyLayer('src/embed.rs')).toBe('infrastructure');
  });

  it('classifies src/daemon.rs as infrastructure', () => {
    expect(classifyLayer('src/daemon.rs')).toBe('infrastructure');
  });

  it('classifies standard hex paths for Rust too', () => {
    expect(classifyLayer('src/core/domain/entities.rs')).toBe('domain');
    expect(classifyLayer('src/core/ports/mod.rs')).toBe('ports');
    expect(classifyLayer('src/adapters/secondary/db.rs')).toBe('adapters/secondary');
  });
});

describe('Layer classifier — Rust special files', () => {
  it('recognizes src/lib.rs as composition root', () => {
    expect(classifySpecialFile('src/lib.rs')).toBe('composition-root');
  });

  it('recognizes src/main.rs as entry point', () => {
    expect(classifySpecialFile('src/main.rs')).toBe('entry-point');
  });

  it('lib.rs and main.rs return unknown from classifyLayer (not a hex layer)', () => {
    expect(classifyLayer('src/lib.rs')).toBe('unknown');
    expect(classifyLayer('src/main.rs')).toBe('unknown');
  });
});

describe('Layer classifier — Go extended conventions', () => {
  it('classifies composition-root.go as composition root', () => {
    expect(classifySpecialFile('src/composition-root.go')).toBe('composition-root');
  });

  it('classifies *_adapter.go as primary adapter', () => {
    expect(classifyLayer('src/http_adapter.go')).toBe('adapters/primary');
  });

  it('classifies *_service.go as usecases', () => {
    expect(classifyLayer('src/weather_service.go')).toBe('usecases');
  });

  it('classifies handler_*.go as primary adapter', () => {
    expect(classifyLayer('src/handler_users.go')).toBe('adapters/primary');
  });

  it('classifies handlers/ directory as primary adapter', () => {
    expect(classifyLayer('handlers/user.go')).toBe('adapters/primary');
  });

  it('excludes _test.go files from classification', () => {
    expect(classifyLayer('internal/service/worker_test.go')).toBe('unknown');
    expect(classifyLayer('cmd/server/main_test.go')).toBe('unknown');
  });
});

describe('Layer classifier — TypeScript patterns still work', () => {
  it('classifies TS domain files', () => {
    expect(classifyLayer('src/core/domain/value-objects.ts')).toBe('domain');
  });

  it('classifies TS ports files', () => {
    expect(classifyLayer('src/core/ports/index.ts')).toBe('ports');
  });

  it('classifies TS primary adapters', () => {
    expect(classifyLayer('src/adapters/primary/cli-adapter.ts')).toBe('adapters/primary');
  });

  it('classifies TS secondary adapters', () => {
    expect(classifyLayer('src/adapters/secondary/git-adapter.ts')).toBe('adapters/secondary');
  });

  it('classifies TS usecases', () => {
    expect(classifyLayer('src/core/usecases/arch-analyzer.ts')).toBe('usecases');
  });

  it('classifies TS infrastructure', () => {
    expect(classifyLayer('src/infrastructure/treesitter-queries.ts')).toBe('infrastructure');
  });
});
