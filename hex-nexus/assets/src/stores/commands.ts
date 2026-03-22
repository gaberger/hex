/**
 * commands.ts — Command registry for the CommandPalette.
 *
 * Each command has an id, label, category, optional shortcut, and action.
 * Commands are registered at app init. The palette fuzzy-searches this list.
 */
import {
  splitPane,
  closePane,
  toggleMaximize,
  focusNextPane,
  focusPrevPane,
  replaceActivePane,
  openPane,
} from "./panes";
import { setSpawnDialogOpen, setSwarmInitDialogOpen, setShortcutsOpen } from "./ui";
import { toggleViewMode } from "./view";
import { addToast } from "./toast";
import { fetchHealth } from "./health";
import { setPanelContent } from "./context-panel";
import { projects } from "./projects";
import { swarms } from "./connection";
import { navigate } from "./router";

export type CommandCategory =
  | "navigation"
  | "project"
  | "agent"
  | "swarm"
  | "inference"
  | "analysis"
  | "session"
  | "view"
  | "settings";

export interface Command {
  id: string;
  label: string;
  category: CommandCategory;
  shortcut?: string;
  action: () => void | Promise<void>;
}

/** All registered commands. */
const commands: Command[] = [
  // ── Navigation ──
  {
    id: "nav.projects",
    label: "Navigate to Projects",
    category: "navigation",
    action: () => navigate({ page: "control-plane" }),
  },
  {
    id: "nav.agents",
    label: "Navigate to Agents",
    category: "navigation",
    action: () => navigate({ page: "agent-fleet" }),
  },
  {
    id: "nav.swarms",
    label: "Navigate to Swarms",
    category: "navigation",
    action: () => navigate({ page: "control-plane" }),
  },
  {
    id: "nav.adrs",
    label: "View ADRs",
    category: "navigation",
    action: () => navigate({ page: "adrs" }),
  },
  {
    id: "nav.workplans",
    label: "View Workplans",
    category: "navigation",
    action: () => navigate({ page: "workplans" }),
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
    action: () => navigate({ page: "fleet-nodes" }),
  },

  // ── View ──
  {
    id: "view.split-h",
    label: "Split Pane Horizontal",
    category: "view",
    shortcut: "Ctrl+\\",
    action: () => splitPane("horizontal"),
  },
  {
    id: "view.split-v",
    label: "Split Pane Vertical",
    category: "view",
    shortcut: "Ctrl+-",
    action: () => splitPane("vertical"),
  },
  {
    id: "view.close",
    label: "Close Pane",
    category: "view",
    shortcut: "Ctrl+W",
    action: () => closePane(),
  },
  {
    id: "view.maximize",
    label: "Toggle Maximize",
    category: "view",
    shortcut: "Ctrl+Shift+Enter",
    action: () => toggleMaximize(),
  },
  {
    id: "view.toggle",
    label: "Toggle Chat / Panes View",
    category: "view",
    shortcut: "Ctrl+Shift+C",
    action: () => toggleViewMode(),
  },
  {
    id: "view.next-pane",
    label: "Focus Next Pane",
    category: "view",
    shortcut: "Ctrl+]",
    action: () => focusNextPane(),
  },
  {
    id: "view.prev-pane",
    label: "Focus Previous Pane",
    category: "view",
    shortcut: "Ctrl+[",
    action: () => focusPrevPane(),
  },
  {
    id: "view.projects",
    label: "Show Project Overview",
    category: "project",
    action: () => replaceActivePane("project-overview", "Projects"),
  },
  {
    id: "view.chat",
    label: "Open Chat",
    category: "session",
    action: () => openPane("chat", "Chat"),
  },

  // ── Agent ──
  {
    id: "agent.spawn",
    label: "Spawn Agent",
    category: "agent",
    shortcut: "Ctrl+N",
    action: () => setSpawnDialogOpen(true),
  },

  // ── Project ──
  {
    id: "project.analyze",
    label: "Run Architecture Analysis",
    category: "project",
    action: async () => {
      try {
        const res = await fetch("/api/analyze", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ path: "." }),
        });
        if (res.ok) {
          const data = await res.json();
          addToast("success", `Analysis complete — Score: ${data.health_score ?? "?"}/100`);
        } else {
          addToast("error", "Analysis failed");
        }
      } catch {
        addToast("error", "Analysis request failed — is nexus running?");
      }
    },
  },

  // ── Health ──
  {
    id: "project.health",
    label: "Show Architecture Health",
    category: "project",
    action: async () => {
      await fetchHealth();
      setPanelContent({ type: "health-detail" });
    },
  },

  // ── Dependency Graph ──
  {
    id: "view.dep-graph",
    label: "Show Dependency Graph",
    category: "view",
    action: () => openPane("dep-graph", "Dependencies"),
  },

  // ── Inference ──
  {
    id: "inference.panel",
    label: "Open Inference Panel",
    category: "inference",
    action: () => openPane("inference", "Inference"),
  },

  // ── Fleet ──
  {
    id: "fleet.view",
    label: "Open Fleet View",
    category: "view",
    action: () => openPane("fleet-view", "Fleet"),
  },

  // ── Swarm ──
  {
    id: "swarm.init",
    label: "Initialize New Swarm",
    category: "swarm",
    action: () => setSwarmInitDialogOpen(true),
  },

  // ── Analysis ──
  {
    id: "analysis.run",
    label: "Run Analysis",
    category: "analysis",
    action: async () => {
      try {
        const res = await fetch("/api/analyze", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ path: "." }),
        });
        if (res.ok) {
          const data = await res.json();
          addToast("success", `Analysis complete — Score: ${data.health_score ?? "?"}/100`);
        } else {
          addToast("error", "Analysis failed");
        }
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
        const res = await fetch("/api/config/sync", { method: "POST" });
        if (res.ok) {
          addToast("success", "Config refreshed from repo");
        } else {
          addToast("error", "Config refresh failed");
        }
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

  // Add swarm navigation
  for (const s of swarms()) {
    const name = s.name ?? s.swarm_name ?? "";
    if (!name) continue;
    cmds.push({
      id: `goto.swarm.${name}`,
      label: `Swarm: ${name}`,
      category: "swarm",
      action: () => navigate({ page: "control-plane" }),
    });
  }

  return cmds;
}

/** Simple fuzzy match: all query chars must appear in order in the target. */
function fuzzyMatch(query: string, target: string): { match: boolean; score: number } {
  const q = query.toLowerCase();
  const t = target.toLowerCase();
  if (q.length === 0) return { match: true, score: 1 };

  let qi = 0;
  let score = 0;
  let prevMatch = false;

  for (let ti = 0; ti < t.length && qi < q.length; ti++) {
    if (t[ti] === q[qi]) {
      score += prevMatch ? 2 : 1; // consecutive chars score higher
      if (ti === 0 || t[ti - 1] === " " || t[ti - 1] === ".") score += 3; // word boundary
      prevMatch = true;
      qi++;
    } else {
      prevMatch = false;
    }
  }

  return { match: qi === q.length, score };
}

/** Search commands by fuzzy query. Returns sorted by relevance. */
export function searchCommands(query: string): Command[] {
  const all = getAllCommandsWithEntities();
  if (!query.trim()) return all;

  return all
    .map((cmd) => {
      const labelMatch = fuzzyMatch(query, cmd.label);
      const catMatch = fuzzyMatch(query, cmd.category);
      const best = labelMatch.score >= catMatch.score ? labelMatch : catMatch;
      return { cmd, ...best };
    })
    .filter((r) => r.match)
    .sort((a, b) => b.score - a.score)
    .map((r) => r.cmd);
}

/** Get all commands (unfiltered), including dynamic entity commands. */
export function getAllCommands(): Command[] {
  return getAllCommandsWithEntities();
}
