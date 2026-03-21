/**
 * projects.ts — Project list store backed by SpacetimeDB subscription.
 *
 * Source: SpacetimeDB `project` table via hexflo-coordination module.
 * Projects are registered via SpacetimeDB reducers, not REST.
 */
import { createMemo } from "solid-js";
import { registeredProjects, getHexfloConn } from "./connection";
import { addToast } from "./toast";

export interface Project {
  id: string;
  name: string;
  path: string;
}

// Reactive project list from SpacetimeDB subscription
export const projects = createMemo<Project[]>(() => {
  return registeredProjects().map((p: any) => ({
    id: p.projectId ?? p.project_id ?? p.id ?? "",
    name: p.name ?? "unnamed",
    path: p.path ?? "",
  }));
});

export async function registerProject(path: string): Promise<boolean> {
  const conn = getHexfloConn();
  if (!conn) {
    addToast("error", "SpacetimeDB not connected — cannot register project");
    return false;
  }
  try {
    const id = path.split("/").pop() || `project-${Date.now()}`;
    const name = id;
    const timestamp = new Date().toISOString();
    conn.reducers.registerProject(id, name, path, timestamp);
    addToast("success", `Project registered: ${path}`);
    return true;
  } catch (err: any) {
    addToast("error", `Register failed: ${err.message}`);
    return false;
  }
}
