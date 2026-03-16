/**
 * Version Adapter — Secondary adapter implementing IVersionPort
 *
 * Resolves CLI version from package.json and hub version from the hex-hub binary.
 */

import { execFile } from 'node:child_process';
import { existsSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { promisify } from 'node:util';
import { Version } from '../../core/ports/index.js';
import type { IVersionPort, VersionInfo } from '../../core/ports/index.js';

const execFileAsync = promisify(execFile);

const HUB_BINARY_PATHS = [
  join(homedir(), '.hex', 'bin', 'hex-hub'),
  join(process.cwd(), 'hex-hub', 'target', 'release', 'hex-hub'),
  join(process.cwd(), 'hex-hub', 'target', 'debug', 'hex-hub'),
];

// ─── Helpers ──────────────────────────────────────────────

function findHubBinary(): string | null {
  for (const p of HUB_BINARY_PATHS) {
    if (existsSync(p)) return p;
  }
  return null;
}

function loadCliVersionString(): string {
  // Walk up from the current file until we find package.json.
  // Works both in source (src/adapters/secondary/) and bundled (dist/) layouts.
  let dir = dirname(fileURLToPath(import.meta.url));
  for (let i = 0; i < 6; i++) {
    const candidate = join(dir, 'package.json');
    if (existsSync(candidate)) {
      try {
        const raw: unknown = JSON.parse(readFileSync(candidate, 'utf-8'));
        if (typeof raw === 'object' && raw !== null && 'version' in raw) {
          return String((raw as Record<string, unknown>).version);
        }
      } catch { /* continue searching */ }
    }
    const parent = dirname(dir);
    if (parent === dir) break;
    dir = parent;
  }
  return '0.0.0';
}

// ─── VersionAdapter ───────────────────────────────────────

export class VersionAdapter implements IVersionPort {
  private readonly cliVersionStr: string;

  constructor() {
    this.cliVersionStr = loadCliVersionString();
  }

  getCliVersion(): Version {
    return Version.parse(this.cliVersionStr) ?? new Version(0, 1, 0);
  }

  async getHubVersion(): Promise<Version | null> {
    const binary = findHubBinary();
    if (!binary) return null;

    // Try running --version first
    try {
      const { stdout } = await execFileAsync(binary, ['--version'], {
        timeout: 3000,
      });
      // Output may be "hex-hub 26.3" or just "26.3.1"
      const match = stdout.trim().match(/(\d+\.\d+(?:\.\d+)?)\s*$/);
      if (match) return Version.parse(match[1]);
    } catch { /* binary may not support --version */ }

    // Fall back to reading Cargo.toml next to the binary's source
    try {
      const cargoPath = join(process.cwd(), 'hex-hub', 'Cargo.toml');
      const cargo = readFileSync(cargoPath, 'utf-8');
      const vMatch = cargo.match(/^version\s*=\s*"([^"]+)"/m);
      if (vMatch) return Version.parse(vMatch[1]);
    } catch { /* no Cargo.toml available */ }

    return null;
  }

  async getVersionInfo(): Promise<VersionInfo> {
    const cli = this.getCliVersion();
    const hubBinaryPath = findHubBinary();
    const hub = await this.getHubVersion();
    const mismatch = hub !== null && !cli.equals(hub);

    return { cli, hub, hubBinaryPath, mismatch };
  }
}
