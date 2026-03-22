/**
 * commands.ts — Command registry for the CommandPalette.
 *
 * Each command has an id, label, category, optional shortcut, and action.
 * Commands are registered at app init. The palette fuzzy-searches this list.
 */
import { navigate, activeProjectId } from "./router";
import { toggleMode } from "./mode";
import { setSpawnDialogOpen, setSwarmInitDialogOpen, setShortcutsOpen } from "./ui";
import { fetchHealth } from "./health";
import { projects } from "./projects";
import { swarms } from "./connection";
import { addToast } from "./toast";
import { restClient } from "../services/rest-client";

export type CommandCategory = "navigation" | "view" | "agent" | "swarm" | "project" | "inference" | "settings" | "session";

export interface Command {
  id: string;
  label: string;
  category: string;
  shortcut?: string;
  action: () => void | Promise<void>;
}

/** Fuzzy search commands by label. */
export function searchCommands(query: string): Command[] {
  if (!query) return getAllCommandsWithEntities();
  const q = query.toLowerCase();
  return getAllCommandsWithEntities().filter(
    (c) => c.label.toLowerCase().includes(q) || c.id.toLowerCase().includes(q)
  );
}

/** Helper: navigate to a project-scoped page using activeProjectId */
function navProject(page: string, extra?: Record<string, string>) {
  const pid = activeProjectId();
  if (!pid) {
    addToast("error", "Select a project first");
    return;
  }
  navigate({ page, projectId: pid, ...extra } as any);
}

const commands: Command[] = [
  // ── Navigation ──
  {
    id: "nav.control-plane",
    label: "Navigate to Control Plane",
    category: "navigation",
    action: () => navigate({ page: "control-plane" }),
  },
  {
    id: "nav.agents",
    label: "Navigate to Agents",
    category: "navigation",
    action: () => navProject("project-agents"),
  },
  {
    id: "nav.swarms",
    label: "Navigate to Swarms",
    category: "navigation",
    action: () => navProject("project-swarms"),
  },
  {
    id: "nav.adrs",
    label: "View ADRs",
    category: "navigation",
    action: () => navProject("project-adrs"),
  },
  {
    id: "nav.workplans",
    label: "View WorkPlans",
    category: "navigation",
    action: () => navProject("project-workplans"),
  },
  {
    id: "nav.inference",
    label: "Navigate to Inference",
    category: "navigation",
    action: () => navigate({ page: "inference" }),
  },
  {
    id: "nav.fleet",
    label: "Navigate to Fleet Nodes",
    category: "navigation",
    action: () => navigate({ page: "fleet" }),
  },
  {
    id: "nav.files",
    label: "Browse Files",
    category: "navigation",
    action: () => navProject("project-files"),
  },
  {
    id: "nav.chat",
    label: "Open Chat",
    category: "navigation",
    action: () => navProject("project-chat"),
  },
  {
    id: "nav.config",
    label: "Open Configuration",
    category: "navigation",
    action: () => navProject("project-config", { section: "blueprint" }),
  },
  {
    id: "nav.health",
    label: "Show Architecture Health",
    category: "navigation",
    action: () => navProject("project-health"),
  },
  {
    id: "nav.graph",
    label: "Show Dependency Graph",
    category: "navigation",
    action: () => navProject("project-graph"),
  },

  // ── Mode ──
  {
    id: "mode.toggle",
    label: "Toggle Plan / Build Mode",
    category: "view",
    shortcut: "Tab",
    action: () => toggleMode(),
  },

  // ── Agent ──
  {
    id: "agent.spawn",
    label: "Spawn Agent",
    category: "agent",
    shortcut: "Ctrl+N",
    action: () => setSpawnDialogOpen(true),
  },

  // ── Swarm ──
  {
    id: "swarm.init",
    label: "Initialize New Swarm",
    category: "swarm",
    action: () => setSwarmInitDialogOpen(true),
  },

  // ── Project ──
  {
    id: "project.analyze",
    label: "Run Architecture Analysis",
    category: "project",
    action: async () => {
      try {
        const data = await restClient.post<any>("/api/analyze", { path: "." });
        addToast("success", `Analysis complete — Score: ${data.health_score ?? "?"}/100`);
      } catch {
        addToast("error", "Analysis request failed — is nexus running?");
      }
    },
  },

  // ── Config ──
  {
    id: "config.refresh",
    label: "Refresh Config",
    category: "settings",
    action: async () => {
      try {
        await restClient.post("/api/config/sync");
        addToast("success", "Config refreshed from repo");
      } catch {
        addToast("error", "Config refresh failed — is nexus running?");
      }
    },
  },

  // ── Settings / Help ──
  {
    id: "help.shortcuts",
    label: "Show Keyboard Shortcuts",
    category: "settings",
    shortcut: "Ctrl+?",
    action: () => setShortcutsOpen(true),
  },
];

/** Returns all commands including dynamic entity-based commands. */
export function getAllCommandsWithEntities(): Command[] {
  const cmds: Command[] = [...commands];

  // Add project navigation
  for (const p of projects()) {
    cmds.push({
      id: `goto.project.${p.id}`,
      label: `Go to ${p.name}`,
      category: "project",
      action: () => navigate({ page: "project", projectId: p.id }),
    });
  }

  // Add swarm navigation (project-scoped)
  for (const s of swarms()) {
    const name = s.name ?? s.swarm_name ?? "";
    const pid = s.project_id ?? activeProjectId();
    if (!name || !pid) continue;
    cmds.push({
      id: `goto.swarm.${name}`,
      label: `Swarm: ${name}`,
      category: "swarm",
      action: () => navigate({ page: "project-swarm-detail", projectId: pid, swarmId: s.id ?? name }),
    });
  }

  return cmds;
}
