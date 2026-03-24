/**
 * router.ts — Project-centric hash routing (ADR-052, ADR-056).
 *
 * Every page has a unique URL via hash routing. Browser back/forward works.
 * Project is THE root entity — all views are scoped under a project.
 * Global views (inference, fleet) aggregate across projects.
 *
 * Memos (activeProjectId, breadcrumbs) created inside initRouterStore() —
 * must be called from App.tsx after initProjectStore() (ADR-2603231000).
 */
import { createSignal, createMemo, createRoot, type Accessor } from "solid-js";
import { projects } from "./projects";

// ── Route types ─────────────────────────────────────────────────────────────

export type Route =
  // Global
  | { page: "control-plane" }
  | { page: "inference" }
  | { page: "fleet" }
  | { page: "research-lab" }
  // Project-scoped
  | { page: "project"; projectId: string }
  | { page: "project-agents"; projectId: string }
  | { page: "project-agent-detail"; projectId: string; agentId: string }
  | { page: "project-swarms"; projectId: string }
  | { page: "project-swarm-detail"; projectId: string; swarmId: string }
  | { page: "project-swarm-task"; projectId: string; swarmId: string; taskId: string }
  | { page: "project-adrs"; projectId: string }
  | { page: "project-adr-detail"; projectId: string; adrId: string }
  | { page: "project-workplans"; projectId: string }
  | { page: "project-workplan-detail"; projectId: string; workplanId: string }
  | { page: "project-health"; projectId: string }
  | { page: "project-graph"; projectId: string }
  | { page: "project-files"; projectId: string }
  | { page: "project-file"; projectId: string; filePath: string }
  | { page: "project-chat"; projectId: string; sessionId?: string }
  | { page: "project-config"; projectId: string; section: string };

// ── State ───────────────────────────────────────────────────────────────────

// Signal is safe at module level — no computation, just a getter/setter pair
const [route, setRoute] = createSignal<Route>({ page: "control-plane" });
export { route };

// ── Derived state (assigned inside createRoot by initRouterStore) ───────────

/** Active project — derived from route. */
let activeProjectId: Accessor<string> = () => "";
export { activeProjectId };

// ── Breadcrumbs ─────────────────────────────────────────────────────────────

export interface Breadcrumb {
  label: string;
  icon: string;
  route?: Route;
}

let breadcrumbs: Accessor<Breadcrumb[]> = () => [];
export { breadcrumbs };

// ── Initialization (call from App.tsx after initProjectStore) ───────────────

let _initialized = false;

export function initRouterStore() {
  if (_initialized) return;
  _initialized = true;

  createRoot(() => {
    activeProjectId = createMemo(() => {
      const r = route();
      return (r as any).projectId ?? "";
    });

    const projectName = (pid: string): string => {
      return ((projects() ?? []).find((p) => p.id === pid)?.name ?? pid) || "Project";
    };

    breadcrumbs = createMemo<Breadcrumb[]>(() => {
      const r = route();
      const crumbs: Breadcrumb[] = [
        { label: "Control Plane", icon: "hexagon", route: { page: "control-plane" } },
      ];

      if (r.page === "control-plane") return crumbs;

      // Global pages
      if (r.page === "inference") {
        crumbs.push({ label: "Inference", icon: "server" });
        return crumbs;
      }
      if (r.page === "fleet") {
        crumbs.push({ label: "Fleet Nodes", icon: "monitor" });
        return crumbs;
      }
      if (r.page === "research-lab") {
        crumbs.push({ label: "Research Lab", icon: "cpu" });
        return crumbs;
      }

      // All remaining pages are project-scoped
      const pid = (r as any).projectId as string;
      if (!pid) return crumbs;

      crumbs.push({
        label: projectName(pid),
        icon: "folder",
        route: { page: "project", projectId: pid },
      });

      switch (r.page) {
        case "project":
          break;

        case "project-agents":
          crumbs.push({ label: "Agents", icon: "bot", route: { page: "project-agents", projectId: pid } });
          break;

        case "project-agent-detail":
          crumbs.push({ label: "Agents", icon: "bot", route: { page: "project-agents", projectId: pid } });
          crumbs.push({ label: r.agentId, icon: "bot" });
          break;

        case "project-swarms":
          crumbs.push({ label: "Swarms", icon: "zap", route: { page: "project-swarms", projectId: pid } });
          break;

        case "project-swarm-detail":
          crumbs.push({ label: "Swarms", icon: "zap", route: { page: "project-swarms", projectId: pid } });
          crumbs.push({ label: r.swarmId, icon: "zap" });
          break;

        case "project-swarm-task":
          crumbs.push({ label: "Swarms", icon: "zap", route: { page: "project-swarms", projectId: pid } });
          crumbs.push({ label: r.swarmId, icon: "zap", route: { page: "project-swarm-detail", projectId: pid, swarmId: r.swarmId } });
          crumbs.push({ label: `Task ${r.taskId}`, icon: "check-square" });
          break;

        case "project-adrs":
          crumbs.push({ label: "ADRs", icon: "file-text", route: { page: "project-adrs", projectId: pid } });
          break;

        case "project-adr-detail":
          crumbs.push({ label: "ADRs", icon: "file-text", route: { page: "project-adrs", projectId: pid } });
          crumbs.push({ label: `ADR-${r.adrId}`, icon: "file-text" });
          break;

        case "project-workplans":
          crumbs.push({ label: "WorkPlans", icon: "clipboard-list", route: { page: "project-workplans", projectId: pid } });
          break;

        case "project-workplan-detail":
          crumbs.push({ label: "WorkPlans", icon: "clipboard-list", route: { page: "project-workplans", projectId: pid } });
          crumbs.push({ label: r.workplanId, icon: "clipboard-list" });
          break;

        case "project-health":
          crumbs.push({ label: "Health", icon: "activity" });
          break;

        case "project-graph":
          crumbs.push({ label: "Dependencies", icon: "share-2" });
          break;

        case "project-files":
          crumbs.push({ label: "Files", icon: "folder" });
          break;

        case "project-file": {
          crumbs.push({ label: "Files", icon: "folder", route: { page: "project-files", projectId: pid } });
          const filename = r.filePath.split("/").pop() || r.filePath;
          crumbs.push({ label: filename, icon: "file" });
          break;
        }

        case "project-chat":
          crumbs.push({ label: r.sessionId || "Chat", icon: "message-square" });
          break;

        case "project-config": {
          const sectionLabels: Record<string, string> = {
            blueprint: "Blueprint",
            tools: "MCP Tools",
            hooks: "Hooks",
            skills: "Skills",
            context: "Context",
            agents: "Agent Definitions",
            spacetimedb: "SpacetimeDB",
          };
          crumbs.push({ label: "Config", icon: "settings", route: { page: "project-config", projectId: pid, section: "blueprint" } });
          if (r.section !== "blueprint") {
            crumbs.push({ label: sectionLabels[r.section] || r.section, icon: "settings" });
          }
          break;
        }
      }

      return crumbs;
    });
  });
}

// ── Navigation ──────────────────────────────────────────────────────────────

export function navigate(newRoute: Route) {
  setRoute(newRoute);
  window.location.hash = routeToHash(newRoute);
}

// ── Hash ↔ Route conversion ─────────────────────────────────────────────────

function routeToHash(r: Route): string {
  switch (r.page) {
    case "control-plane":
      return "#/";
    case "inference":
      return "#/inference";
    case "fleet":
      return "#/fleet";
    case "research-lab":
      return "#/research-lab";
    case "project":
      return `#/project/${r.projectId}`;
    case "project-agents":
      return `#/project/${r.projectId}/agents`;
    case "project-agent-detail":
      return `#/project/${r.projectId}/agents/${r.agentId}`;
    case "project-swarms":
      return `#/project/${r.projectId}/swarms`;
    case "project-swarm-detail":
      return `#/project/${r.projectId}/swarms/${r.swarmId}`;
    case "project-swarm-task":
      return `#/project/${r.projectId}/swarms/${r.swarmId}/tasks/${r.taskId}`;
    case "project-adrs":
      return `#/project/${r.projectId}/adrs`;
    case "project-adr-detail":
      return `#/project/${r.projectId}/adrs/${r.adrId}`;
    case "project-workplans":
      return `#/project/${r.projectId}/workplans`;
    case "project-workplan-detail":
      return `#/project/${r.projectId}/workplans/${r.workplanId}`;
    case "project-health":
      return `#/project/${r.projectId}/health`;
    case "project-graph":
      return `#/project/${r.projectId}/graph`;
    case "project-files":
      return `#/project/${r.projectId}/files`;
    case "project-file":
      return `#/project/${r.projectId}/files/${encodeURIComponent(r.filePath)}`;
    case "project-chat":
      return r.sessionId
        ? `#/project/${r.projectId}/chat/${r.sessionId}`
        : `#/project/${r.projectId}/chat`;
    case "project-config":
      return `#/project/${r.projectId}/config/${r.section}`;
    default:
      return "#/";
  }
}

function hashToRoute(hash: string): Route {
  const path = hash.replace("#", "") || "/";
  const parts = path.split("/").filter(Boolean);

  // Global routes
  if (parts[0] === "inference") return { page: "inference" };
  if (parts[0] === "fleet") return { page: "fleet" };
  if (parts[0] === "research-lab") return { page: "research-lab" };

  // Project-scoped routes: /project/:id/...
  if (parts[0] === "project" && parts[1]) {
    const projectId = decodeURIComponent(parts[1]);
    const sub = parts[2];

    if (!sub) return { page: "project", projectId };

    switch (sub) {
      case "agents":
        if (parts[3]) return { page: "project-agent-detail", projectId, agentId: decodeURIComponent(parts[3]) };
        return { page: "project-agents", projectId };

      case "swarms":
        if (parts[3] && parts[4] === "tasks" && parts[5]) {
          return { page: "project-swarm-task", projectId, swarmId: decodeURIComponent(parts[3]), taskId: decodeURIComponent(parts[5]) };
        }
        if (parts[3]) return { page: "project-swarm-detail", projectId, swarmId: decodeURIComponent(parts[3]) };
        return { page: "project-swarms", projectId };

      case "adrs":
        if (parts[3]) return { page: "project-adr-detail", projectId, adrId: decodeURIComponent(parts[3]) };
        return { page: "project-adrs", projectId };

      case "workplans":
        if (parts[3]) return { page: "project-workplan-detail", projectId, workplanId: decodeURIComponent(parts[3]) };
        return { page: "project-workplans", projectId };

      case "health":
        return { page: "project-health", projectId };

      case "graph":
        return { page: "project-graph", projectId };

      case "files":
        if (parts[3]) {
          const filePath = decodeURIComponent(parts.slice(3).join("/"));
          return { page: "project-file", projectId, filePath };
        }
        return { page: "project-files", projectId };

      case "chat":
        return { page: "project-chat", projectId, sessionId: parts[3] };

      case "config":
        return { page: "project-config", projectId, section: parts[3] || "blueprint" };

      default:
        return { page: "project", projectId };
    }
  }

  return { page: "control-plane" };
}

// ── Initialization ──────────────────────────────────────────────────────────

/** Initialize router — call once at app startup */
export function initRouter() {
  const initial = hashToRoute(window.location.hash);
  setRoute(initial);

  window.addEventListener("hashchange", () => {
    setRoute(hashToRoute(window.location.hash));
  });
}
