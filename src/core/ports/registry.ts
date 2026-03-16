/**
 * Project Registry Port
 *
 * Tracks hex-intf projects globally so the hub dashboard can
 * discover them and each project gets a stable unique ID + port.
 * Registry lives at ~/.hex-intf/registry.json (outside any project).
 */

import type { ProjectRegistration, LocalProjectIdentity } from '../domain/value-objects.js';

// Re-export domain types for adapters (adapters import from ports, not domain)
export type { ProjectRegistration, LocalProjectIdentity, ProjectRegistry } from '../domain/value-objects.js';

export interface IRegistryPort {
  /** Register a project. Assigns a UUID and port if new. */
  register(rootPath: string, name: string): Promise<ProjectRegistration>;

  /** Remove a project from the registry. */
  unregister(projectId: string): Promise<boolean>;

  /** List all registered projects. */
  list(): Promise<ProjectRegistration[]>;

  /** Find a project by its root path. */
  findByPath(rootPath: string): Promise<ProjectRegistration | null>;

  /** Update lastSeenAt for a project. */
  touch(projectId: string): Promise<void>;

  /** Read the local project identity from .hex-intf/project.json */
  readLocalIdentity(rootPath: string): Promise<LocalProjectIdentity | null>;

  /** Write the local project identity to .hex-intf/project.json */
  writeLocalIdentity(rootPath: string, identity: LocalProjectIdentity): Promise<void>;
}
