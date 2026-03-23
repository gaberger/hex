/**
 * projects.ts — Project list store backed by SpacetimeDB subscription.
 *
 * ALL state operations go through SpacetimeDB reducers via WebSocket.
 * REST is ONLY used for filesystem operations (scaffold, file deletion)
 * because WASM modules cannot access the local filesystem.
 *
 * Reactive primitives created inside initProjectStore() — must be called
 * from App.tsx composition root after initConnectionStore() (ADR-2603231000).
 */
import { createSignal, createEffect, createRoot, type Accessor } from "solid-js";
import { registeredProjects, getHexfloConn } from "./connection";
import { addToast } from "./toast";
import { restClient } from "../services/rest-client";

export interface Project {
  id: string;
  name: string;
  path: string;
  health?: "green" | "yellow" | "red";
  lastActivity?: string;
}

// Stable signal proxy — never reassigned, so SolidJS JSX compiler always
// tracks the same accessor. The createEffect inside initProjectStore()
// pushes updates into this signal when SpacetimeDB subscription data changes.
const [projects, setProjects] = createSignal<Project[]>([]);
export { projects };

// ── Initialization ───────────────────────────────────────────────────────

let _initialized = false;

export function initProjectStore() {
  if (_initialized) return;
  _initialized = true;

  createRoot(() => {
    createEffect(() => {
      const mapped = (registeredProjects() ?? [])
        .map((p: any) => ({
          id: p.projectId ?? p.project_id ?? p.id ?? "",
          name: p.name ?? "unnamed",
          path: p.path ?? "",
        }))
        .filter((p: Project) => p.id !== "");
      setProjects(mapped);
    });
  });
}

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
    initResult = await restClient.post<InitResult>("/api/projects/init", { path });
    if (initResult && initResult.created.length > 0) {
      addToast(
        "success",
        `Scaffolded ${initResult.created.length} file(s): ${initResult.created.join(", ")}`,
      );
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
    console.log("[projects] removeProject reducer call with id:", id);
    conn.reducers.removeProject(id);
    addToast("success", "Project unregistered");
    return true;
  } catch (err: any) {
    console.error("[projects] removeProject failed:", err);
    addToast("error", `Unregister failed: ${err.message}`);
    return false;
  }
}

/**
 * Archive a project — delete config files, then remove from SpacetimeDB.
 * Order: filesystem first (needs project path from state), reducer second (UI update).
 */
export async function archiveProject(
  id: string,
  removeClaude = false,
): Promise<boolean> {
  // Get path from local state BEFORE removing from SpacetimeDB
  const project = projects().find((p) => p.id === id);
  const path = project?.path ?? "";

  // 1. Delete config files from disk (REST — filesystem op)
  try {
    await restClient.post(`/api/projects/${id}/archive`, { removeClaude, path });
  } catch (e: any) {
    // 404 is acceptable (nothing to archive), other errors are non-fatal
    if (!e.message?.includes("404")) {
      // Filesystem cleanup failed — still remove from state
    }
  }

  // 2. Remove from SpacetimeDB (instant UI update via subscription)
  const conn = getHexfloConn();
  if (conn) {
    try { conn.reducers.removeProject(id); } catch { /* ignore */ }
  }

  addToast("success", "Project archived — config removed, source files preserved");
  return true;
}

/**
 * Delete a project — delete ALL files from disk, then remove from SpacetimeDB.
 * Order: filesystem first (needs project path from state), reducer second (UI update).
 */
export async function deleteProject(id: string): Promise<boolean> {
  // Get path from local state BEFORE removing from SpacetimeDB
  const project = projects().find((p) => p.id === id);
  const path = project?.path ?? "";

  // 1. Delete all files from disk (REST — filesystem op)
  try {
    await restClient.post(`/api/projects/${id}/delete`, { confirm: true, path });
  } catch (err: any) {
    addToast("error", `Delete failed: ${err.message}`);
    return false;
  }

  // 2. Remove from SpacetimeDB (instant UI update via subscription)
  const conn = getHexfloConn();
  if (conn) {
    try { conn.reducers.removeProject(id); } catch { /* ignore */ }
  }

  addToast("success", "Project deleted permanently");
  return true;
}
