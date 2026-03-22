/**
 * projects.ts — Project list store backed by SpacetimeDB subscription.
 *
 * ALL state operations go through SpacetimeDB reducers via WebSocket.
 * REST is ONLY used for filesystem operations (scaffold, file deletion)
 * because WASM modules cannot access the local filesystem.
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

/** Scaffold project config, then register in SpacetimeDB. */
export async function registerProject(path: string): Promise<boolean> {
  // Step 1: Scaffold .hex/, .claude/, docs/adrs/, CLAUDE.md (REST — filesystem op)
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

  // Step 2: Register via SpacetimeDB reducer (WebSocket)
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

/** Unregister a project (keeps all files). SpacetimeDB reducer only. */
export async function unregisterProject(id: string): Promise<boolean> {
  const conn = getHexfloConn();
  if (!conn) {
    addToast("error", "SpacetimeDB not connected");
    return false;
  }
  try {
    conn.reducers.removeProject(id);
    addToast("success", "Project unregistered");
    return true;
  } catch (err: any) {
    addToast("error", `Unregister failed: ${err.message}`);
    return false;
  }
}

/**
 * Archive a project — remove from SpacetimeDB + delete config files from disk.
 * Reducer: SpacetimeDB (state). REST: hex-nexus (filesystem cleanup).
 */
export async function archiveProject(
  id: string,
  removeClaude = false,
): Promise<boolean> {
  // 1. Remove from SpacetimeDB (instant UI update via subscription)
  const conn = getHexfloConn();
  if (conn) {
    try {
      conn.reducers.removeProject(id);
    } catch {
      // Continue — REST fallback will also unregister
    }
  }

  // 2. Delete config files from disk (REST — filesystem op, WASM can't do this)
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
    // 404 means the archive endpoint isn't available — that's fine, state is already removed
    if (res.status === 404) {
      addToast("success", "Project unregistered (config files need manual cleanup)");
      return true;
    }
    const err = await res.json().catch(() => ({}));
    addToast("error", `Archive failed: ${err.error ?? res.statusText}`);
    return false;
  } catch (err: any) {
    // SpacetimeDB removal succeeded even if REST failed
    addToast("warning", "Project unregistered but config cleanup failed — remove .hex/ manually");
    return true;
  }
}

/**
 * Delete a project — remove from SpacetimeDB + delete ALL files from disk.
 * Reducer: SpacetimeDB (state). REST: hex-nexus (filesystem deletion).
 */
export async function deleteProject(id: string): Promise<boolean> {
  // 1. Remove from SpacetimeDB (instant UI update via subscription)
  const conn = getHexfloConn();
  if (conn) {
    try {
      conn.reducers.removeProject(id);
    } catch {
      // Continue — REST will also handle state removal
    }
  }

  // 2. Delete all files from disk (REST — filesystem op, WASM can't do this)
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
    addToast("error", `File deletion failed: ${err.error ?? res.statusText}`);
    return false;
  } catch (err: any) {
    addToast("error", `Delete failed: ${err.message}`);
    return false;
  }
}
