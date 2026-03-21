import { createSignal, createMemo } from "solid-js";

export type Route =
  | { page: "control-plane" }
  | { page: "project"; projectId: string }
  | { page: "project-chat"; projectId: string; sessionId?: string }
  | { page: "project-adr"; projectId: string; adrId: string }
  | { page: "project-health"; projectId: string }
  | { page: "project-graph"; projectId: string }
  | { page: "agent-fleet" }
  | { page: "config"; section: string }
  | { page: "inference" }
  | { page: "fleet-nodes" };

const [route, setRoute] = createSignal<Route>({ page: "control-plane" });
export { route };

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
    crumbs.push({
      label: "Configuration",
      icon: "settings",
      route: { page: "config", section: "blueprint" },
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
  } else if (r.page === "inference") {
    crumbs.push({ label: "Inference", icon: "server" });
  } else if (r.page === "fleet-nodes") {
    crumbs.push({ label: "Fleet Nodes", icon: "monitor" });
  }

  return crumbs;
});

export function navigate(newRoute: Route) {
  setRoute(newRoute);
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
    case "agent-fleet":
      return "#/agents";
    case "config":
      return `#/config/${r.section}`;
    case "inference":
      return "#/inference";
    case "fleet-nodes":
      return "#/fleet";
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
    if (parts[2] === "adr" && parts[3])
      return { page: "project-adr", projectId, adrId: decodeURIComponent(parts[3]) };
    if (parts[2] === "health")
      return { page: "project-health", projectId };
    if (parts[2] === "graph")
      return { page: "project-graph", projectId };
    return { page: "project", projectId };
  }
  if (parts[0] === "agents") return { page: "agent-fleet" };
  if (parts[0] === "config")
    return { page: "config", section: parts[1] || "blueprint" };
  if (parts[0] === "inference") return { page: "inference" };
  if (parts[0] === "fleet") return { page: "fleet-nodes" };

  return { page: "control-plane" };
}

/** Initialize router — call once at app startup */
export function initRouter() {
  setRoute(hashToRoute(window.location.hash));

  window.addEventListener("hashchange", () => {
    setRoute(hashToRoute(window.location.hash));
  });
}
