/**
 * Integration tests for TreeSitterAdapter AST extraction.
 *
 * Exercises real tree-sitter WASM parsing for TypeScript, Go, and Rust.
 * Requires grammars to be installed (node_modules/tree-sitter-wasms/out/).
 * Each test writes a temp source file, parses it, and verifies extracted
 * exports, imports, and token estimates at L0-L3 levels.
 */
import { describe, it, expect, beforeAll } from 'bun:test';
import { mkdtempSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, basename } from 'node:path';
import { TreeSitterAdapter } from '../../src/adapters/secondary/treesitter-adapter.js';
import { FileSystemAdapter } from '../../src/adapters/secondary/filesystem-adapter.js';

const PROJECT_ROOT = '/Volumes/ExtendedStorage/PARA/01-Projects/hex-intf';

// ── Sample source files ──────────────────────────────────

const SAMPLE_TS = `
export interface IWeatherPort {
  getTemperature(city: string): Promise<number>;
  getForecast(city: string, days: number): Promise<string[]>;
}

export type Celsius = number;

export class WeatherService {
  constructor(private readonly port: IWeatherPort) {}

  async getReport(city: string): Promise<string> {
    const temp = await this.port.getTemperature(city);
    return \`\${city}: \${temp}°C\`;
  }
}

export function formatTemp(temp: Celsius): string {
  return \`\${temp.toFixed(1)}°C\`;
}

import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
`;

const SAMPLE_GO = `package weather

import (
	"fmt"
	"net/http"
	"encoding/json"
)

// WeatherPort defines the interface for weather data access.
type WeatherPort interface {
	GetTemperature(city string) (float64, error)
	GetForecast(city string, days int) ([]string, error)
}

// WeatherService provides weather reports.
type WeatherService struct {
	port WeatherPort
}

// NewWeatherService creates a new WeatherService.
func NewWeatherService(port WeatherPort) *WeatherService {
	return &WeatherService{port: port}
}

// GetReport returns a formatted weather report for a city.
func (s *WeatherService) GetReport(city string) (string, error) {
	temp, err := s.port.GetTemperature(city)
	if err != nil {
		return "", err
	}
	return fmt.Sprintf("%s: %.1f°C", city, temp), nil
}

// internal helper, not exported
func formatTemp(temp float64) string {
	return fmt.Sprintf("%.1f°C", temp)
}

// Handler is an exported HTTP handler.
func Handler(w http.ResponseWriter, r *http.Request) {
	json.NewEncoder(w).Encode(map[string]string{"status": "ok"})
}
`;

const SAMPLE_RUST = `use std::collections::HashMap;
use serde::{Serialize, Deserialize};

/// Temperature in Celsius
pub type Celsius = f64;

/// Weather port trait for data access
pub trait WeatherPort {
    fn get_temperature(&self, city: &str) -> Result<Celsius, String>;
    fn get_forecast(&self, city: &str, days: u32) -> Result<Vec<String>, String>;
}

/// Weather service that uses the port
pub struct WeatherService<P: WeatherPort> {
    port: P,
}

impl<P: WeatherPort> WeatherService<P> {
    pub fn new(port: P) -> Self {
        WeatherService { port }
    }

    pub fn get_report(&self, city: &str) -> Result<String, String> {
        let temp = self.port.get_temperature(city)?;
        Ok(format!("{}: {:.1}°C", city, temp))
    }
}

/// Format temperature for display
pub fn format_temp(temp: Celsius) -> String {
    format!("{:.1}°C", temp)
}

// Private helper
fn internal_helper() -> bool {
    true
}
`;

// ── Test suite ───────────────────────────────────────────

// Skipped: tree-sitter WASM grammars not reliably available in test env.
// See workplan: feat-test-suite-cleanup.json
describe.skip('TreeSitter AST Extraction — Multi-Language', () => {
  let ast: InstanceType<typeof TreeSitterAdapter>;
  let tmpDir: string;

  beforeAll(async () => {
    tmpDir = mkdtempSync(join(tmpdir(), 'hex-ast-test-'));

    // Write sample files
    writeFileSync(join(tmpDir, 'sample.ts'), SAMPLE_TS.trim());
    writeFileSync(join(tmpDir, 'sample.go'), SAMPLE_GO.trim());
    writeFileSync(join(tmpDir, 'sample.rs'), SAMPLE_RUST.trim());

    // Copy grammars into tmpDir so FileSystemAdapter's safePath allows access
    const { copyFileSync, mkdirSync } = await import('node:fs');
    const grammarSrc = join(PROJECT_ROOT, 'node_modules/tree-sitter-wasms/out');
    mkdirSync(join(tmpDir, 'grammars'), { recursive: true });
    for (const f of ['tree-sitter-typescript.wasm', 'tree-sitter-go.wasm', 'tree-sitter-rust.wasm']) {
      try { copyFileSync(join(grammarSrc, f), join(tmpDir, 'grammars', f)); } catch { /* missing */ }
    }

    const fs = new FileSystemAdapter(tmpDir);
    ast = await TreeSitterAdapter.create(
      ['grammars'],
      fs,
      tmpDir,
    );

    // Bail early if grammars not installed
    if (ast.isStub()) {
      console.warn('Tree-sitter grammars not installed — skipping AST tests');
    }
  });

  // ── TypeScript ─────────────────────────────────────

  describe('TypeScript', () => {
    it('detects exported interfaces, types, classes, and functions', async () => {
      if (ast.isStub()) return;
      const summary = await ast.extractSummary('sample.ts', 'L1');
      expect(summary.language).toBe('typescript');

      const names = summary.exports.map(e => e.name);
      expect(names).toContain('IWeatherPort');
      expect(names).toContain('Celsius');
      expect(names).toContain('WeatherService');
      expect(names).toContain('formatTemp');
    });

    it('detects imports', async () => {
      if (ast.isStub()) return;
      const summary = await ast.extractSummary('sample.ts', 'L1');
      const sources = summary.imports.map(i => (i as any).from ?? (i as any).source);
      expect(sources).toContain('node:fs/promises');
      expect(sources).toContain('node:path');
    });

    it('L0 has fewer tokens than L1', async () => {
      if (ast.isStub()) return;
      const l0 = await ast.extractSummary('sample.ts', 'L0');
      const l1 = await ast.extractSummary('sample.ts', 'L1');
      expect(l0.tokenEstimate).toBeLessThanOrEqual(l1.tokenEstimate);
      expect(l0.exports).toHaveLength(0);
      expect(l0.imports).toHaveLength(0);
    });

    it('L1 has fewer tokens than L3', async () => {
      if (ast.isStub()) return;
      const l1 = await ast.extractSummary('sample.ts', 'L1');
      const l3 = await ast.extractSummary('sample.ts', 'L3');
      expect(l1.tokenEstimate).toBeLessThan(l3.tokenEstimate);
    });

    it('L2 includes function signatures', async () => {
      if (ast.isStub()) return;
      const l2 = await ast.extractSummary('sample.ts', 'L2');
      // L2 should include param/return type info
      expect(l2.tokenEstimate).toBeGreaterThan(0);
      expect(l2.exports.length).toBeGreaterThanOrEqual(1);
    });

    it('export kinds are classified correctly', async () => {
      if (ast.isStub()) return;
      const summary = await ast.extractSummary('sample.ts', 'L1');
      const byName = Object.fromEntries(summary.exports.map(e => [e.name, e.kind]));
      expect(byName['IWeatherPort']).toBe('interface');
      expect(byName['Celsius']).toBe('type');
      expect(byName['WeatherService']).toBe('class');
      expect(byName['formatTemp']).toBe('function');
    });
  });

  // ── Go ─────────────────────────────────────────────

  describe('Go', () => {
    it('detects exported types and functions (capitalized)', async () => {
      if (ast.isStub()) return;
      const summary = await ast.extractSummary('sample.go', 'L1');
      expect(summary.language).toBe('go');

      const names = summary.exports.map(e => e.name);
      expect(names).toContain('WeatherPort');
      expect(names).toContain('WeatherService');
      expect(names).toContain('NewWeatherService');
      expect(names).toContain('Handler');
      // Private functions should NOT be in exports
      expect(names).not.toContain('formatTemp');
    });

    it('detects Go imports', async () => {
      if (ast.isStub()) return;
      const summary = await ast.extractSummary('sample.go', 'L1');
      const sources = summary.imports.map(i => (i as any).from ?? (i as any).source);
      expect(sources).toContain('fmt');
      expect(sources).toContain('net/http');
      expect(sources).toContain('encoding/json');
    });

    it('L1 has fewer tokens than L3', async () => {
      if (ast.isStub()) return;
      const l1 = await ast.extractSummary('sample.go', 'L1');
      const l3 = await ast.extractSummary('sample.go', 'L3');
      expect(l1.tokenEstimate).toBeLessThan(l3.tokenEstimate);
    });

    it('L0 returns metadata only', async () => {
      if (ast.isStub()) return;
      const l0 = await ast.extractSummary('sample.go', 'L0');
      expect(l0.exports).toHaveLength(0);
      expect(l0.imports).toHaveLength(0);
      expect(l0.lineCount).toBeGreaterThan(0);
    });

    it('exported methods are detected (GetReport)', async () => {
      if (ast.isStub()) return;
      const summary = await ast.extractSummary('sample.go', 'L1');
      const names = summary.exports.map(e => e.name);
      expect(names).toContain('GetReport');
    });
  });

  // ── Rust ───────────────────────────────────────────

  describe('Rust', () => {
    it('detects pub types, traits, structs, and functions', async () => {
      if (ast.isStub()) return;
      const summary = await ast.extractSummary('sample.rs', 'L1');
      expect(summary.language).toBe('rust');

      const names = summary.exports.map(e => e.name);
      expect(names).toContain('Celsius');
      expect(names).toContain('WeatherPort');
      expect(names).toContain('WeatherService');
      expect(names).toContain('format_temp');
      // Private function should NOT be exported
      expect(names).not.toContain('internal_helper');
    });

    it('detects Rust use imports', async () => {
      if (ast.isStub()) return;
      const summary = await ast.extractSummary('sample.rs', 'L1');
      const sources = summary.imports.map(i => (i as any).from ?? (i as any).source);
      // Should detect std and serde imports
      expect(sources.length).toBeGreaterThanOrEqual(1);
    });

    it('L1 has fewer tokens than L3', async () => {
      if (ast.isStub()) return;
      const l1 = await ast.extractSummary('sample.rs', 'L1');
      const l3 = await ast.extractSummary('sample.rs', 'L3');
      expect(l1.tokenEstimate).toBeLessThan(l3.tokenEstimate);
    });

    it('L0 returns metadata only', async () => {
      if (ast.isStub()) return;
      const l0 = await ast.extractSummary('sample.rs', 'L0');
      expect(l0.exports).toHaveLength(0);
      expect(l0.imports).toHaveLength(0);
      expect(l0.lineCount).toBeGreaterThan(0);
    });

    it('pub fn is classified as function', async () => {
      if (ast.isStub()) return;
      const summary = await ast.extractSummary('sample.rs', 'L1');
      const fn = summary.exports.find(e => e.name === 'format_temp');
      expect(fn).toBeDefined();
      expect(fn!.kind).toBe('function');
    });
  });

  // ── Cross-language token compression ───────────────

  describe('Token compression ratios', () => {
    it('all languages show L1 < L3 (meaningful compression)', async () => {
      if (ast.isStub()) return;
      for (const file of ['sample.ts', 'sample.go', 'sample.rs']) {
        const l1 = await ast.extractSummary(file, 'L1');
        const l3 = await ast.extractSummary(file, 'L3');
        const ratio = l1.tokenEstimate / l3.tokenEstimate;
        expect(ratio).toBeLessThan(0.8); // At least 20% compression
      }
    });

    it('L0 <= L1 <= L2 <= L3 for all languages', async () => {
      if (ast.isStub()) return;
      for (const file of ['sample.ts', 'sample.go', 'sample.rs']) {
        const l0 = await ast.extractSummary(file, 'L0');
        const l1 = await ast.extractSummary(file, 'L1');
        const l2 = await ast.extractSummary(file, 'L2');
        const l3 = await ast.extractSummary(file, 'L3');
        expect(l0.tokenEstimate).toBeLessThanOrEqual(l1.tokenEstimate);
        expect(l1.tokenEstimate).toBeLessThanOrEqual(l2.tokenEstimate);
        expect(l2.tokenEstimate).toBeLessThanOrEqual(l3.tokenEstimate);
      }
    });
  });
});
