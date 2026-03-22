/**
 * project-match.ts — Canonical project matching utility.
 *
 * SpacetimeDB projects have IDs like "hex-intf-1xq8wun" (name + random suffix).
 * But swarms, agents, and other entities may reference projects by:
 *   - Full ID: "hex-intf-1xq8wun"
 *   - Name only: "hex-intf"
 *   - Root path: "/Volumes/.../hex-intf"
 *   - Empty string (unscoped)
 *
 * This utility normalizes matching so all components use the same logic.
 * EVERY component that filters by project MUST use this function.
 */

import { projects } from "../stores/projects";

export interface ProjectInfo {
  id: string;
  name: string;
  rootPath: string;
}

/** Get project info by ID from the projects store. */
export function getProjectInfo(projectId: string): ProjectInfo | undefined {
  const p = projects().find((p: any) => p.id === projectId);
  if (!p) return undefined;
  return {
    id: p.id,
    name: (p as any).name ?? "",
    rootPath: (p as any).rootPath ?? (p as any).path ?? "",
  };
}

/**
 * Check if an entity's project reference matches a given project ID.
 *
 * Handles all known reference formats:
 *   - Exact ID match ("hex-intf-1xq8wun" === "hex-intf-1xq8wun")
 *   - Name prefix match ("hex-intf" is prefix of "hex-intf-1xq8wun")
 *   - ID starts with entity ref ("hex-intf-1xq8wun" starts with "hex-intf")
 *   - Path contains project name ("/path/to/hex-intf" contains "hex-intf")
 *   - Empty ref matches nothing (unscoped entities don't belong to any project)
 *
 * @param entityRef - The project reference on the entity (project_id, projectId, projectDir)
 * @param projectId - The canonical project ID to match against
 * @returns true if the entity belongs to this project
 */
export function matchesProject(entityRef: string, projectId: string): boolean {
  if (!entityRef || !projectId) return false;

  // Exact match
  if (entityRef === projectId) return true;

  // Look up project info for fuzzy matching
  const info = getProjectInfo(projectId);
  if (!info) return entityRef === projectId;

  // Name-based matching (entity stores name, project has ID with suffix)
  if (info.name && entityRef === info.name) return true;
  if (info.name && projectId.startsWith(entityRef)) return true;

  // Path-based matching (agent stores projectDir)
  if (info.rootPath && entityRef === info.rootPath) return true;
  if (info.rootPath && info.name && entityRef.includes(info.name)) return true;

  return false;
}

/**
 * Extract project reference from any entity shape.
 * Handles both snake_case and camelCase field names.
 */
export function getEntityProjectRef(entity: any): string {
  return entity.project_id ?? entity.projectId ?? entity.project ?? "";
}

/**
 * Extract project directory from an agent entity.
 */
export function getAgentProjectDir(agent: any): string {
  return agent.project_dir ?? agent.projectDir ?? agent.projectDirectory ?? "";
}

/**
 * Check if an entity belongs to a project, checking all reference fields.
 */
export function entityBelongsToProject(entity: any, projectId: string): boolean {
  const ref = getEntityProjectRef(entity);
  if (matchesProject(ref, projectId)) return true;

  // Also check projectDir for agents
  const dir = getAgentProjectDir(entity);
  if (dir && matchesProject(dir, projectId)) return true;

  return false;
}
