import { createSignal, createMemo, createEffect } from "solid-js";

export type Route =
  | { page: "control-plane" }
  | { page: "project"; projectId: string }
  | { page: "project-chat"; projectId: string; sessionId?: string }
  | { page: "project-adr"; projectId: string; adrId: string }
  | { page: "project-health"; projectId: string }
  | { page: "project-graph"; projectId: string }
  | { page: "agent-fleet" }
  | { page: "config"; section: string; projectId?: string }
  | { page: "adrs"; projectId?: string }
  | { page: "inference" }
  | { page: "fleet-nodes" }
  | { page: "file-viewer"; filePath: string; projectId?: string }
  | { page: "file-tree"; projectId?: string }
  | { page: "workplans" };

const [route, setRoute] = createSignal<Route>({ page: "control-plane" });
export { route };

/** Active project — persists across route changes. Set when navigating to any project-scoped page. */
const [activeProjectId, setActiveProjectId] = createSignal<string>("");
export { activeProjectId, setActiveProjectId };

export interface Breadcrumb {
  label: string;
  icon: string; // lucide icon name
  route?: Route; // if clickable
}

export const breadcrumbs = createMemo<Breadcrumb[]>(() => {
  const r = route();
  const crumbs: Breadcrumb[] = [
    {
      label: "Control Plane",
      icon: "hexagon",
      route: { page: "control-plane" },
    },
  ];

  if (r.page === "control-plane") return crumbs;

  if (r.page.startsWith("project")) {
    const pid = (r as Extract<Route, { projectId: string }>).projectId ?? "";
    crumbs.push({
      label: pid || "Project",
      icon: "folder",
      route: { page: "project", projectId: pid },
    });

    if (r.page === "project-chat") {
      const sessionId = (r as Extract<Route, { page: "project-chat" }>)
        .sessionId;
      crumbs.push({ label: sessionId || "Chat", icon: "message-square" });
    } else if (r.page === "project-adr") {
      const adrId = (r as Extract<Route, { page: "project-adr" }>).adrId;
      crumbs.push({ label: `ADR-${adrId}`, icon: "file-text" });
    } else if (r.page === "project-health") {
      crumbs.push({ label: "Health", icon: "activity" });
    } else if (r.page === "project-graph") {
      crumbs.push({ label: "Dependencies", icon: "share-2" });
    }
  } else if (r.page === "agent-fleet") {
    crumbs.push({ label: "Agent Fleet", icon: "bot" });
  } else if (r.page === "config") {
    const configPid = (r as any).projectId;
    if (configPid) {
      crumbs.push({ label: configPid, icon: "folder", route: { page: "project", projectId: configPid } });
    }
    crumbs.push({
      label: "Configuration",
      icon: "settings",
      route: { page: "config", section: "blueprint", projectId: configPid },
    });
    const section = (r as Extract<Route, { page: "config" }>).section;
    if (section && section !== "blueprint") {
      const labels: Record<string, string> = {
        blueprint: "Architecture Blueprint",
        tools: "MCP Tools",
        hooks: "Hooks",
        skills: "Skills",
        context: "Context (CLAUDE.md)",
        agents: "Agent Definitions",
        spacetimedb: "SpacetimeDB",
      };
      crumbs.push({ label: labels[section] || section, icon: "settings" });
    }
  } else if (r.page === "adrs") {
    crumbs.push({ label: "ADRs", icon: "file-text" });
  } else if (r.page === "inference") {
    crumbs.push({ label: "Inference", icon: "server" });
  } else if (r.page === "fleet-nodes") {
    crumbs.push({ label: "Fleet Nodes", icon: "monitor" });
  } else if (r.page === "workplans") {
    crumbs.push({ label: "Workplans", icon: "clipboard-list" });
  } else if (r.page === "file-tree") {
    crumbs.push({ label: "Files", icon: "folder" });
  } else if (r.page === "file-viewer") {
    const fp = (r as Extract<Route, { page: "file-viewer" }>).filePath ?? "";
    const filename = fp.split("/").pop() || fp;
    crumbs.push({ label: filename, icon: "file-text" });
  }

  return crumbs;
});

export function navigate(newRoute: Route) {
  setRoute(newRoute);
  // Track active project across route changes
  const pid = (newRoute as any).projectId;
  if (pid) setActiveProjectId(pid);
  const hash = routeToHash(newRoute);
  window.location.hash = hash;
}

function routeToHash(r: Route): string {
  switch (r.page) {
    case "control-plane":
      return "#/";
    case "project":
      return `#/project/${r.projectId}`;
    case "project-chat":
      return `#/project/${r.projectId}/chat`;
    case "project-adr":
      return `#/project/${r.projectId}/adr/${r.adrId}`;
    case "project-health":
      return `#/project/${r.projectId}/health`;
    case "project-graph":
      return `#/project/${r.projectId}/graph`;
    case "adrs":
      return r.projectId ? `#/project/${r.projectId}/adrs` : "#/adrs";
    case "agent-fleet":
      return "#/agents";
    case "config":
      return r.projectId ? `#/project/${r.projectId}/config/${r.section}` : `#/config/${r.section}`;
    case "inference":
      return "#/inference";
    case "fleet-nodes":
      return "#/fleet";
    case "file-tree":
      return "#/files";
    case "workplans":
      return "#/workplans";
    case "file-viewer":
      return `#/file/${encodeURIComponent(r.filePath)}`;
    default:
      return "#/";
  }
}

function hashToRoute(hash: string): Route {
  const path = hash.replace("#", "") || "/";
  const parts = path.split("/").filter(Boolean);

  if (parts[0] === "project" && parts[1]) {
    const projectId = decodeURIComponent(parts[1]);
    if (parts[2] === "chat")
      return { page: "project-chat", projectId };
    if (parts[2] === "adrs")
      return { page: "adrs", projectId };
    if (parts[2] === "adr" && parts[3])
      return { page: "project-adr", projectId, adrId: decodeURIComponent(parts[3]) };
    if (parts[2] === "health")
      return { page: "project-health", projectId };
    if (parts[2] === "graph")
      return { page: "project-graph", projectId };
    if (parts[2] === "config")
      return { page: "config", section: parts[3] || "blueprint", projectId };
    return { page: "project", projectId };
  }
  if (parts[0] === "adrs") return { page: "adrs" };
  if (parts[0] === "files") return { page: "file-tree" };
  if (parts[0] === "file" && parts[1]) {
    const filePath = decodeURIComponent(parts.slice(1).join("/"));
    return { page: "file-viewer", filePath };
  }
  if (parts[0] === "agents") return { page: "agent-fleet" };
  if (parts[0] === "config")
    return { page: "config", section: parts[1] || "blueprint" };
  if (parts[0] === "inference") return { page: "inference" };
  if (parts[0] === "fleet") return { page: "fleet-nodes" };
  if (parts[0] === "workplans") return { page: "workplans" };

  return { page: "control-plane" };
}

/** Initialize router — call once at app startup */
export function initRouter() {
  const initial = hashToRoute(window.location.hash);
  setRoute(initial);
  const pid = (initial as any).projectId;
  if (pid) setActiveProjectId(pid);

  window.addEventListener("hashchange", () => {
    const r = hashToRoute(window.location.hash);
    setRoute(r);
    const p = (r as any).projectId;
    if (p) setActiveProjectId(p);
  });
}
