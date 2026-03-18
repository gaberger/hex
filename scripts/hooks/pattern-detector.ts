#!/usr/bin/env bun
/**
 * Pattern Detector — Memory-based Pre-Commit Validation (Phase 1)
 *
 * Detects "nullable-but-always-initialized" anti-pattern by analyzing:
 * 1. composition-root.ts: What ports are ALWAYS created
 * 2. app-context.ts: What ports are typed as | null
 * 3. Cross-reference: Warn when mismatch detected
 *
 * This prevents bugs like the ADR tracking false positive (commit a274139).
 */

import { readFileSync } from 'fs';
import { join } from 'path';

interface PatternViolation {
  portName: string;
  issue: string;
  file: string;
  line?: number;
  suggestion: string;
}

/**
 * Extract port names that are ALWAYS initialized in composition-root.ts
 * Looks for: const X = new Y(...) followed by return { X, ... }
 */
function extractAlwaysInitializedPorts(rootPath: string): Set<string> {
  const compositionPath = join(rootPath, 'src/composition-root.ts');
  const content = readFileSync(compositionPath, 'utf-8');

  const alwaysInitialized = new Set<string>();

  // Find the main AppContext return statement
  const returnMatch = content.match(/return\s*{([^}]+)}/s);
  if (!returnMatch) return alwaysInitialized;

  const returnBlock = returnMatch[1];

  // Extract all port names from return { x, y, z, ... }
  // Handles both shorthand (x) and explicit (x: x) syntax
  const portNames = returnBlock
    .split(',')
    .map(line => line.trim())
    .filter(line => line && !line.startsWith('//'))
    .map(line => {
      // Handle "x: x" or just "x"
      const colonMatch = line.match(/^(\w+)\s*:/);
      if (colonMatch) return colonMatch[1];

      const simpleMatch = line.match(/^(\w+)$/);
      if (simpleMatch) return simpleMatch[1];

      return null;
    })
    .filter((name): name is string => name !== null);

  // Now check: is each port ALWAYS created (no conditional)?
  // Look for: const portName = ... (not inside if/try blocks)
  for (const portName of portNames) {
    // Simple heuristic: if "const portName" appears outside conditionals, it's always created
    const constRegex = new RegExp(`const\\s+${portName}\\s*=`, 'g');
    const matches = [...content.matchAll(constRegex)];

    if (matches.length === 0) continue;

    // Check if declaration is inside an if/try block (rough heuristic)
    for (const match of matches) {
      const pos = match.index!;
      const before = content.slice(0, pos);

      // Count open/close braces and if/try keywords
      const openBraces = (before.match(/{/g) || []).length;
      const closeBraces = (before.match(/}/g) || []).length;
      const ifCount = (before.match(/\bif\s*\(/g) || []).length;
      const tryCount = (before.match(/\btry\s*{/g) || []).length;

      // If at top level (balanced braces) and no recent if/try, it's always initialized
      if (openBraces === closeBraces && ifCount === 0 && tryCount === 0) {
        alwaysInitialized.add(portName);
      }
    }
  }

  return alwaysInitialized;
}

/**
 * Extract port names typed as | null in AppContext
 */
function extractNullablePorts(rootPath: string): Map<string, number> {
  const appContextPath = join(rootPath, 'src/core/ports/app-context.ts');
  const content = readFileSync(appContextPath, 'utf-8');
  const lines = content.split('\n');

  const nullablePorts = new Map<string, number>();

  // Look for: portName: Type | null
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const match = line.match(/^\s*(\w+)\s*:\s*[^;]+\|\s*null\s*;/);
    if (match) {
      const portName = match[1];
      nullablePorts.set(portName, i + 1); // Line numbers start at 1
    }
  }

  return nullablePorts;
}

/**
 * Detect nullable-but-always-initialized anti-pattern
 */
function detectNullableButAlwaysInitialized(rootPath: string): PatternViolation[] {
  const violations: PatternViolation[] = [];

  const alwaysInitialized = extractAlwaysInitializedPorts(rootPath);
  const nullablePorts = extractNullablePorts(rootPath);

  // Cross-reference: ports that are BOTH always-initialized AND nullable
  for (const [portName, line] of nullablePorts) {
    if (alwaysInitialized.has(portName)) {
      violations.push({
        portName,
        issue: `${portName} is typed as | null but is ALWAYS initialized in composition-root.ts`,
        file: 'src/core/ports/app-context.ts',
        line,
        suggestion: `Remove | null from line ${line} — this creates defensive null checks that always fail`,
      });
    }
  }

  return violations;
}

/**
 * CLI entry point
 */
function main() {
  const rootPath = process.cwd();

  console.log('🔍 Scanning for nullable-but-always-initialized anti-pattern...\n');

  const violations = detectNullableButAlwaysInitialized(rootPath);

  if (violations.length === 0) {
    console.log('✅ No nullable-but-always-initialized violations found\n');
    process.exit(0);
  }

  console.log(`⚠️  Found ${violations.length} violation(s):\n`);

  for (const v of violations) {
    console.log(`  ${v.file}:${v.line || '?'}`);
    console.log(`    Issue: ${v.issue}`);
    console.log(`    Fix: ${v.suggestion}\n`);
  }

  console.log('This pattern was learned from commit a274139 (ADR tracking fix).\n');
  console.log('To fix: Update AppContext type to match composition-root reality.\n');

  process.exit(1);
}

// Run if invoked directly
if (import.meta.main) {
  main();
}
