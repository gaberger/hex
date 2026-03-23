import { createSignal } from "solid-js";
import { addToast } from "./toast";
import { restClient } from "../services/rest-client";

export interface HealthViolation {
  file: string;
  message: string;
  severity: string;
}

export interface UnusedPort {
  name: string;
  file: string;
}

export interface HealthData {
  health_score: number;
  file_count: number;
  export_count: number;
  violation_count: number;
  unused_port_count: number;
  dead_export_count: number;
  circular_dep_count: number;
  edge_count: number;
  violations: HealthViolation[];
  unused_ports: UnusedPort[];
  orphan_files?: string[];
  circular_deps?: string[][];
}

const [healthData, setHealthData] = createSignal<HealthData | null>(null);
const [healthLoading, setHealthLoading] = createSignal(false);

export { healthData, healthLoading };

import { fetchProjects } from "./projects";
import { activeProjectId } from "./router";

/** Resolve the best project path to analyze. */
async function resolveProjectPath(): Promise<string> {
  const projs = await fetchProjects();
  if (projs.length > 0) {
    const active = activeProjectId();
    const match = active ? projs.find((p: any) => p.id === active || p.name === active) : null;
    return (match?.path || projs[0].path) || ".";
  }
  // Fallback: try the nexus health endpoint for project info
  try {
    const data = await restClient.get<any>("/api/health");
    if (data.project_dir) return data.project_dir;
  } catch { /* fall through */ }
  return ".";
}

export async function fetchHealth(rootPath?: string) {
  setHealthLoading(true);
  try {
    const path = rootPath || await resolveProjectPath();
    const data = await restClient.post<any>("/api/analyze", { root_path: path });
    if (data.error) {
      addToast("error", `Analysis failed: ${data.error}`);
      return null;
    }
    setHealthData(data as HealthData);
    return data as HealthData;
  } catch (e) {
    console.error("[health] fetch failed:", e);
    addToast("error", "Analysis request failed — is nexus running?");
  } finally {
    setHealthLoading(false);
  }
  return null;
}
