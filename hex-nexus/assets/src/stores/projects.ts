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
  health?: "green" | "yellow" | "red";
  lastActivity?: string;
}

// Reactive project list from SpacetimeDB subscription
export const projects = createMemo<Project[]>(() => {
  return registeredProjects().map((p: any) => ({
    id: p.projectId ?? p.project_id ?? p.id ?? "",
    name: p.name ?? "unnamed",
    path: p.path ?? "",
  }));
});

/** Return current projects from SpacetimeDB subscription. */
export function fetchProjects(): Project[] {
  return projects();
}

export interface InitResult {
  initialized: boolean;
  name: string;
  path: string;
  created: string[];
}

/** Unregister a project from nexus (keeps all files). */
export async function unregisterProject(id: string): Promise<boolean> {
  try {
    const res = await fetch(`/api/projects/${id}`, { method: "DELETE" });
    if (res.ok) {
      addToast("success", "Project unregistered");
      return true;
    }
    const err = await res.json().catch(() => ({}));
    addToast("error", `Unregister failed: ${err.error ?? res.statusText}`);
    return false;
  } catch (err: any) {
    addToast("error", `Unregister failed: ${err.message}`);
    return false;
  }
}

/** Archive a project — unregister + remove .hex/ config, keep source files. */
export async function archiveProject(
  id: string,
  removeClaude = false,
): Promise<boolean> {
  try {
    const res = await fetch(`/api/projects/${id}/archive`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ removeClaude }),
    });
    if (res.ok) {
      addToast("success", "Project archived — config removed, source files preserved");
      return true;
    }
    // Fallback: if archive endpoint doesn't exist yet, just unregister
    if (res.status === 404) {
      return unregisterProject(id);
    }
    const err = await res.json().catch(() => ({}));
    addToast("error", `Archive failed: ${err.error ?? res.statusText}`);
    return false;
  } catch (err: any) {
    addToast("error", `Archive failed: ${err.message}`);
    return false;
  }
}

/** Delete a project — unregister + delete ALL files from disk. Requires confirmation. */
export async function deleteProject(id: string): Promise<boolean> {
  try {
    const res = await fetch(`/api/projects/${id}/delete`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ confirm: true }),
    });
    if (res.ok) {
      addToast("success", "Project deleted permanently");
      return true;
    }
    const err = await res.json().catch(() => ({}));
    addToast("error", `Delete failed: ${err.error ?? res.statusText}`);
    return false;
  } catch (err: any) {
    addToast("error", `Delete failed: ${err.message}`);
    return false;
  }
}

/** Scaffold project config, then register in SpacetimeDB. */
export async function registerProject(path: string): Promise<boolean> {
  // Step 1: Scaffold .hex/, .claude/, docs/adrs/, CLAUDE.md
  let initResult: InitResult | null = null;
  try {
    const res = await fetch("/api/projects/init", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path }),
    });
    if (res.ok) {
      initResult = await res.json();
      if (initResult && initResult.created.length > 0) {
        addToast(
          "success",
          `Scaffolded ${initResult.created.length} file(s): ${initResult.created.join(", ")}`,
        );
      }
    }
  } catch {
    // Non-fatal — continue with registration even if scaffold fails
  }

  // Step 2: Register in SpacetimeDB
  const conn = getHexfloConn();
  if (!conn) {
    addToast("error", "SpacetimeDB not connected — cannot register project");
    return false;
  }
  try {
    const name = initResult?.name ?? path.split("/").pop() ?? `project-${Date.now()}`;
    const id = path.split("/").pop() || `project-${Date.now()}`;
    const timestamp = new Date().toISOString();
    conn.reducers.registerProject(id, name, path, timestamp);
    addToast("success", `Project registered: ${path}`);
    return true;
  } catch (err: any) {
    addToast("error", `Register failed: ${err.message}`);
    return false;
  }
}
