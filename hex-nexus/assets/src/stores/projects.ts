/**
 * projects.ts — Project list store backed by SpacetimeDB with REST fallback.
 *
 * Primary source: SpacetimeDB `project` table subscription (reactive).
 * Fallback: REST /api/projects when SpacetimeDB is not connected.
 */
import { createMemo } from "solid-js";
import { registeredProjects, getHexfloConn, hexfloConnected } from "./connection";
import { addToast } from "./toast";

export interface Project {
  id: string;
  name: string;
  path: string;
}

// Primary: reactive signal from SpacetimeDB subscription
export const projects = createMemo<Project[]>(() => {
  return registeredProjects().map((p: any) => ({
    id: p.projectId ?? p.project_id ?? p.id ?? "",
    name: p.name ?? "unnamed",
    path: p.path ?? "",
  }));
});

export async function registerProject(path: string): Promise<boolean> {
  const conn = getHexfloConn();
  if (conn) {
    try {
      const id = path.split("/").pop() || `project-${Date.now()}`;
      const name = id;
      const timestamp = new Date().toISOString();
      conn.reducers.registerProject(id, name, path, timestamp);
      addToast("success", `Project registered: ${path}`);
      return true;
    } catch (err: any) {
      addToast("error", `Register failed: ${err.message}`);
    }
  } else {
    // REST fallback
    try {
      const res = await fetch("/api/projects/register", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ path }),
      });
      if (res.ok) {
        addToast("success", `Project registered: ${path}`);
        return true;
      }
    } catch {}
  }
  return false;
}

// Keep fetchProjects for components that need imperative refresh
export async function fetchProjects(): Promise<Project[]> {
  // If SpacetimeDB is connected, just return the subscription data
  if (hexfloConnected()) {
    return projects();
  }
  // REST fallback
  try {
    const res = await fetch("/api/projects");
    if (res.ok) {
      const data = await res.json();
      return (data.projects ?? data ?? []).map((p: any) => ({
        id: p.id ?? p.name ?? "",
        name: p.name ?? "unnamed",
        path: p.path ?? p.root_path ?? "",
      }));
    }
  } catch {}
  return [];
}
