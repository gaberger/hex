/**
 * Registry Adapter
 *
 * Implements IRegistryPort using the filesystem.
 * Global registry: ~/.hex/registry.json
 * Local identity: <project>/.hex/project.json
 *
 * Port range: 3848-3947 (100 slots), 3847 reserved for hub.
 */

import { readFile, writeFile, mkdir, rename } from 'node:fs/promises';
import { mkdirSync, rmdirSync } from 'node:fs';
import { join } from 'node:path';
import { homedir } from 'node:os';
import { randomUUID } from 'node:crypto';
import type {
  IRegistryPort,
  ProjectRegistration,
  LocalProjectIdentity,
  ProjectRegistry,
} from '../../core/ports/registry.js';

const REGISTRY_DIR = join(homedir(), '.hex');
const REGISTRY_PATH = join(REGISTRY_DIR, 'registry.json');
const LOCK_PATH = REGISTRY_PATH + '.lock';
const TMP_PATH = REGISTRY_PATH + '.tmp';
const PORT_MIN = 3848;
const PORT_MAX = 3947;
const LOCK_RETRIES = 5;
const LOCK_RETRY_MS = 100;

export class RegistryAdapter implements IRegistryPort {
  async register(rootPath: string, name: string): Promise<ProjectRegistration> {
    return this.withLock(async () => {
      const registry = await this.readRegistry();
      const existing = registry.projects.find((p) => p.rootPath === rootPath);
      if (existing) {
        existing.lastSeenAt = Date.now();
        existing.name = name;
        existing.status = 'active';
        await this.writeRegistry(registry);
        return existing;
      }

      const port = this.allocatePort(registry);
      const reg: ProjectRegistration = {
        id: randomUUID(),
        name,
        rootPath,
        port,
        status: 'active',
        createdAt: Date.now(),
        lastSeenAt: Date.now(),
      };

      registry.projects.push(reg);
      await this.writeRegistry(registry);
      return reg;
    });
  }

  async unregister(projectId: string): Promise<boolean> {
    return this.withLock(async () => {
      const registry = await this.readRegistry();
      const idx = registry.projects.findIndex((p) => p.id === projectId);
      if (idx === -1) return false;
      registry.projects.splice(idx, 1);
      await this.writeRegistry(registry);
      return true;
    });
  }

  async list(): Promise<ProjectRegistration[]> {
    const registry = await this.readRegistry();
    return registry.projects;
  }

  async findByPath(rootPath: string): Promise<ProjectRegistration | null> {
    const registry = await this.readRegistry();
    return registry.projects.find((p) => p.rootPath === rootPath) ?? null;
  }

  async touch(projectId: string): Promise<void> {
    return this.withLock(async () => {
      const registry = await this.readRegistry();
      const project = registry.projects.find((p) => p.id === projectId);
      if (project) {
        project.lastSeenAt = Date.now();
        await this.writeRegistry(registry);
      }
    });
  }

  async readLocalIdentity(rootPath: string): Promise<LocalProjectIdentity | null> {
    try {
      const content = await readFile(join(rootPath, '.hex', 'project.json'), 'utf-8');
      const parsed: unknown = JSON.parse(content);
      if (typeof parsed !== 'object' || parsed === null || !('id' in parsed)) {
        return null;
      }
      return parsed as LocalProjectIdentity;
    } catch {
      // project.json doesn't exist yet — project has no local identity
      return null;
    }
  }

  async writeLocalIdentity(rootPath: string, identity: LocalProjectIdentity): Promise<void> {
    const dir = join(rootPath, '.hex');
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, 'project.json'), JSON.stringify(identity, null, 2) + '\n');
  }

  // ─── Private ─────────────────────────────────────────

  private allocatePort(registry: ProjectRegistry): number {
    const usedPorts = new Set(registry.projects.map((p) => p.port));
    for (let port = PORT_MIN; port <= PORT_MAX; port++) {
      if (!usedPorts.has(port)) return port;
    }
    throw new Error(`No available ports in range ${PORT_MIN}-${PORT_MAX}. Unregister unused projects.`);
  }

  private async readRegistry(): Promise<ProjectRegistry> {
    try {
      const content = await readFile(REGISTRY_PATH, 'utf-8');
      const parsed: unknown = JSON.parse(content);
      if (typeof parsed !== 'object' || parsed === null || !('version' in parsed)) {
        return { version: 1, projects: [] };
      }
      return parsed as ProjectRegistry;
    } catch {
      // Registry file doesn't exist or is corrupted — start fresh
      return { version: 1, projects: [] };
    }
  }

  private async writeRegistry(registry: ProjectRegistry): Promise<void> {
    await mkdir(REGISTRY_DIR, { recursive: true });
    await writeFile(TMP_PATH, JSON.stringify(registry, null, 2) + '\n');
    await rename(TMP_PATH, REGISTRY_PATH);
  }

  private acquire(): void {
    // Ensure parent directory exists before attempting lock
    mkdirSync(REGISTRY_DIR, { recursive: true });
    for (let i = 0; i < LOCK_RETRIES; i++) {
      try {
        mkdirSync(LOCK_PATH);
        return;
      } catch (err: unknown) {
        // EEXIST means another process holds the lock — retry
        // Any other error (permissions, etc.) should fail immediately
        const code = (err as NodeJS.ErrnoException).code;
        if (code !== 'EEXIST') throw err;
        if (i === LOCK_RETRIES - 1) {
          throw new Error(`Failed to acquire registry lock after ${LOCK_RETRIES} attempts`);
        }
        Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, LOCK_RETRY_MS);
      }
    }
  }

  private release(): void {
    try {
      rmdirSync(LOCK_PATH);
    } catch {
      // Lock dir already removed — safe to ignore
    }
  }

  private async withLock<T>(fn: () => Promise<T>): Promise<T> {
    this.acquire();
    try {
      return await fn();
    } finally {
      this.release();
    }
  }
}
