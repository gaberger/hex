/**
 * Registry Adapter
 *
 * Implements IRegistryPort using the filesystem.
 * Global registry: ~/.hex-intf/registry.json
 * Local identity: <project>/.hex-intf/project.json
 *
 * Port range: 3848-3947 (100 slots), 3847 reserved for hub.
 */

import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { homedir } from 'node:os';
import { randomUUID } from 'node:crypto';
import type {
  IRegistryPort,
  ProjectRegistration,
  LocalProjectIdentity,
  ProjectRegistry,
} from '../../core/ports/registry.js';

const REGISTRY_DIR = join(homedir(), '.hex-intf');
const REGISTRY_PATH = join(REGISTRY_DIR, 'registry.json');
const PORT_MIN = 3848;
const PORT_MAX = 3947;

export class RegistryAdapter implements IRegistryPort {
  async register(rootPath: string, name: string): Promise<ProjectRegistration> {
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
  }

  async unregister(projectId: string): Promise<boolean> {
    const registry = await this.readRegistry();
    const idx = registry.projects.findIndex((p) => p.id === projectId);
    if (idx === -1) return false;
    registry.projects.splice(idx, 1);
    await this.writeRegistry(registry);
    return true;
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
    const registry = await this.readRegistry();
    const project = registry.projects.find((p) => p.id === projectId);
    if (project) {
      project.lastSeenAt = Date.now();
      await this.writeRegistry(registry);
    }
  }

  async readLocalIdentity(rootPath: string): Promise<LocalProjectIdentity | null> {
    try {
      const content = await readFile(join(rootPath, '.hex-intf', 'project.json'), 'utf-8');
      return JSON.parse(content) as LocalProjectIdentity;
    } catch {
      return null;
    }
  }

  async writeLocalIdentity(rootPath: string, identity: LocalProjectIdentity): Promise<void> {
    const dir = join(rootPath, '.hex-intf');
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
      return JSON.parse(content) as ProjectRegistry;
    } catch {
      return { version: 1, projects: [] };
    }
  }

  private async writeRegistry(registry: ProjectRegistry): Promise<void> {
    await mkdir(REGISTRY_DIR, { recursive: true });
    await writeFile(REGISTRY_PATH, JSON.stringify(registry, null, 2) + '\n');
  }
}
