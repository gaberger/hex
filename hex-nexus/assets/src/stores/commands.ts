/**
 * commands.ts — Command registry for the CommandPalette.
 *
 * Each command has an id, label, category, optional shortcut, and action.
 * Commands are registered at app init. The palette fuzzy-searches this list.
 *
 * Transport rule (ADR-046): State reads use SpacetimeDB signals.
 * State writes use SpacetimeDB reducers. REST only for filesystem ops.
 */
import { navigate, activeProjectId } from "./router";
import { toggleMode } from "./mode";
import { setSpawnDialogOpen, setSwarmInitDialogOpen, setShortcutsOpen } from "./ui";
import { fetchHealth } from "./health";
import { projects, registerProject } from "./projects";
import { swarms, swarmTasks, registryAgents, hexfloMemory, inferenceProviders, getHexfloConn } from "./connection";
import { addToast } from "./toast";
import { restClient } from "../services/rest-client";
import { recordCommandStart, recordCommandSuccess, recordCommandError } from "./command-history";

export type CommandCategory = "navigation" | "view" | "agent" | "swarm" | "project" | "inference" | "settings" | "session" | "analysis" | "task" | "memory" | "inbox" | "git";

export interface Command {
  id: string;
  label: string;
  category: string;
  shortcut?: string;
  action: () => void | Promise<void>;
}

/** Wrap an async command action with history tracking. */
function tracked(label: string, category: string, fn: () => Promise<string | void>): () => Promise<void> {
  return async () => {
    const id = recordCommandStart(label, category);
    try {
      const result = await fn();
      recordCommandSuccess(id, typeof result === "string" ? result : undefined);
    } catch (e: any) {
      recordCommandError(id, e.message ?? "Unknown error");
      throw e;
    }
  };
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
  { id: "nav.control-plane", label: "Navigate to Control Plane", category: "navigation", action: () => navigate({ page: "control-plane" }) },
  { id: "nav.agents", label: "Navigate to Agents", category: "navigation", action: () => navProject("project-agents") },
  { id: "nav.swarms", label: "Navigate to Swarms", category: "navigation", action: () => navProject("project-swarms") },
  { id: "nav.adrs", label: "View ADRs", category: "navigation", action: () => navProject("project-adrs") },
  { id: "nav.workplans", label: "View WorkPlans", category: "navigation", action: () => navProject("project-workplans") },
  { id: "nav.inference", label: "Navigate to Inference", category: "navigation", action: () => navigate({ page: "inference" }) },
  { id: "nav.fleet", label: "Navigate to Fleet Nodes", category: "navigation", action: () => navigate({ page: "fleet" }) },
  { id: "nav.inbox", label: "Open Inbox", category: "navigation", action: () => navProject("project-inbox") },
  { id: "nav.files", label: "Browse Files", category: "navigation", action: () => navProject("project-files") },
  { id: "nav.chat", label: "Open Chat", category: "navigation", action: () => navProject("project-chat") },
  { id: "nav.config", label: "Open Configuration", category: "navigation", action: () => navProject("project-config", { section: "blueprint" }) },
  { id: "nav.health", label: "Show Architecture Health", category: "navigation", action: () => navProject("project-health") },
  { id: "nav.graph", label: "Show Dependency Graph", category: "navigation", action: () => navProject("project-graph") },

  // ── Mode ──
  { id: "mode.toggle", label: "Toggle Plan / Build Mode", category: "view", shortcut: "Tab", action: () => toggleMode() },

  // ── Agent ──
  { id: "agent.spawn", label: "Spawn Agent", category: "agent", shortcut: "Ctrl+N", action: () => setSpawnDialogOpen(true) },
  {
    id: "agent.list",
    label: "List Connected Agents",
    category: "agent",
    action: tracked("List Connected Agents", "agent", async () => {
      const agents = registryAgents();
      const active = agents.filter((a: any) => a.status === "active" || a.status === "connected" || a.status === "online").length;
      addToast("success", `${active} active / ${agents.length} total agent(s)`);
      navProject("project-agents");
      return `${active} active / ${agents.length} total`;
    }),
  },
  {
    id: "agent.cleanup",
    label: "Disconnect All Stale Agents",
    category: "agent",
    action: tracked("Disconnect Stale Agents", "agent", async () => {
      const conn = getHexfloConn();
      if (!conn) { addToast("error", "SpacetimeDB not connected"); throw new Error("Not connected"); }
      const threshold = new Date(Date.now() - 120_000).toISOString();
      conn.reducers.agentEvictDead(threshold);
      addToast("success", "Stale agents evicted");
      return "Evicted";
    }),
  },

  // ── Swarm ──
  { id: "swarm.init", label: "Initialize New Swarm", category: "swarm", action: () => setSwarmInitDialogOpen(true) },
  {
    id: "swarm.status",
    label: "Show Swarm Status",
    category: "swarm",
    action: tracked("Swarm Status", "swarm", async () => {
      const swarmList = swarms();
      const active = swarmList.filter((s: any) => s.status === "active").length;
      const msg = `${active} active / ${swarmList.length} total`;
      addToast("success", msg);
      navProject("project-swarms");
      return msg;
    }),
  },

  // ── Task ──
  {
    id: "task.list",
    label: "List All Tasks",
    category: "task",
    action: tracked("List All Tasks", "task", async () => {
      const tasks = swarmTasks();
      const swarmCount = swarms().length;
      const pending = tasks.filter((t: any) => t.status === "pending").length;
      const msg = `${tasks.length} task(s) across ${swarmCount} swarm(s) — ${pending} pending`;
      addToast("success", msg);
      navProject("project-swarms");
      return msg;
    }),
  },

  // ── Analysis (REST — filesystem ops) ──
  {
    id: "analysis.run",
    label: "Run Architecture Analysis",
    category: "analysis",
    action: tracked("Run Architecture Analysis", "analysis", async () => {
      const data = await restClient.post<any>("/api/analyze", { path: "." });
      const msg = `Score: ${data.health_score ?? "?"}/100`;
      addToast("success", `Analysis complete — ${msg}`);
      return msg;
    }),
  },
  {
    id: "analysis.adr-compliance",
    label: "Run ADR Compliance Check",
    category: "analysis",
    action: tracked("ADR Compliance Check", "analysis", async () => {
      const data = await restClient.post<any>("/api/analyze/adr-compliance", { path: "." });
      const msg = `${data.compliant_count ?? "?"}/${data.total_count ?? "?"} compliant`;
      addToast("success", `ADR compliance: ${msg}`);
      return msg;
    }),
  },
  { id: "analysis.health", label: "Show Architecture Health", category: "analysis", action: () => navProject("project-health") },
  { id: "analysis.graph", label: "Show Dependency Graph", category: "analysis", action: () => navProject("project-graph") },

  // ── Memory (SpacetimeDB) ──
  {
    id: "memory.search",
    label: "Search Memory",
    category: "memory",
    action: tracked("Search Memory", "memory", async () => {
      const query = prompt("Search query:");
      if (!query) return;
      const q = query.toLowerCase();
      const all = hexfloMemory();
      const results = all.filter((m: any) =>
        (m.key ?? "").toLowerCase().includes(q) || (m.value ?? "").toLowerCase().includes(q)
      );
      addToast("success", `${results.length} memory result(s) for "${query}"`);
      return `${results.length} result(s)`;
    }),
  },
  {
    id: "memory.store",
    label: "Store Memory Key-Value",
    category: "memory",
    action: tracked("Store Memory", "memory", async () => {
      const conn = getHexfloConn();
      if (!conn) { addToast("error", "SpacetimeDB not connected"); throw new Error("Not connected"); }
      const key = prompt("Key:");
      if (!key) return;
      const value = prompt("Value:");
      if (!value) return;
      conn.reducers.memoryStore(key, value, "global", new Date().toISOString());
      addToast("success", `Stored memory: ${key}`);
      return `Stored: ${key}`;
    }),
  },
  {
    id: "memory.get",
    label: "Retrieve Memory by Key",
    category: "memory",
    action: tracked("Retrieve Memory", "memory", async () => {
      const key = prompt("Key to retrieve:");
      if (!key) return;
      const entry = hexfloMemory().find((m: any) => m.key === key);
      if (entry) {
        const val = String(entry.value ?? "").slice(0, 100);
        addToast("success", `${key} = ${val}`);
        return `${key} = ${val}`;
      } else {
        addToast("error", `Memory key "${key}" not found`);
        throw new Error(`Key "${key}" not found`);
      }
    }),
  },

  // ── Inbox (SpacetimeDB) ──
  {
    id: "inbox.list",
    label: "Check Notification Inbox",
    category: "inbox",
    action: tracked("Check Inbox", "inbox", async () => {
      const agents = registryAgents();
      addToast("success", `${agents.length} agent(s) — check Inbox page for notifications`);
      navProject("project-inbox");
      return `${agents.length} agent(s)`;
    }),
  },
  {
    id: "inbox.notify",
    label: "Send Notification",
    category: "inbox",
    action: tracked("Send Notification", "inbox", async () => {
      const conn = getHexfloConn();
      if (!conn) { addToast("error", "SpacetimeDB not connected"); throw new Error("Not connected"); }
      const message = prompt("Notification message:");
      if (!message) return;
      const target = prompt("Target agent ID (or 'all'):");
      const timestamp = new Date().toISOString();
      if (!target || target === "all") {
        const pid = activeProjectId() ?? "global";
        conn.reducers.notifyAllAgents(pid, 1, "command", message, timestamp);
      } else {
        conn.reducers.notifyAgent(target, 1, "command", message, timestamp);
      }
      addToast("success", "Notification sent");
      return `Sent to ${target || "all"}`;
    }),
  },

  // ── Project ──
  {
    id: "project.register",
    label: "Register New Project",
    category: "project",
    action: tracked("Register Project", "project", async () => {
      const path = prompt("Project path:");
      if (!path) return;
      const ok = await registerProject(path);
      if (!ok) throw new Error("Registration failed");
      return `Registered: ${path}`;
    }),
  },
  {
    id: "project.status",
    label: "Project Status Overview",
    category: "project",
    action: () => {
      const pid = activeProjectId();
      if (pid) navigate({ page: "project", projectId: pid });
      else navigate({ page: "control-plane" });
    },
  },

  // ── Inference (SpacetimeDB signal) ──
  {
    id: "inference.providers",
    label: "List Inference Providers",
    category: "inference",
    action: tracked("List Inference Providers", "inference", async () => {
      const providers = inferenceProviders();
      addToast("success", `${providers.length} provider(s) configured`);
      navigate({ page: "inference" });
      return `${providers.length} provider(s)`;
    }),
  },

  // ── Git (REST — filesystem ops) ──
  {
    id: "git.status",
    label: "Git Status",
    category: "git",
    action: tracked("Git Status", "git", async () => {
      const pid = activeProjectId();
      if (!pid) { addToast("error", "Select a project first"); throw new Error("No project"); }
      const data = await restClient.get<any>(`/api/${pid}/git/status`);
      const changed = data.data?.changed_files ?? data.data?.files ?? [];
      addToast("success", `${changed.length} changed file(s)`);
      return `${changed.length} changed file(s)`;
    }),
  },
  {
    id: "git.log",
    label: "Git Log (Recent Commits)",
    category: "git",
    action: tracked("Git Log", "git", async () => {
      const pid = activeProjectId();
      if (!pid) { addToast("error", "Select a project first"); throw new Error("No project"); }
      const data = await restClient.get<any>(`/api/${pid}/git/log?limit=10`);
      const commits = data.data?.commits ?? [];
      addToast("success", `${commits.length} recent commit(s)`);
      return `${commits.length} commit(s)`;
    }),
  },
  {
    id: "git.branches",
    label: "Git Branches",
    category: "git",
    action: tracked("Git Branches", "git", async () => {
      const pid = activeProjectId();
      if (!pid) { addToast("error", "Select a project first"); throw new Error("No project"); }
      const data = await restClient.get<any>(`/api/${pid}/git/branches`);
      const branches = data.data?.branches ?? [];
      const head = branches.find((b: any) => b.isHead);
      const msg = `${branches.length} branch(es) — current: ${head?.name ?? "?"}`;
      addToast("success", msg);
      return msg;
    }),
  },

  // ── Config (REST — filesystem sync) ──
  {
    id: "config.refresh",
    label: "Refresh Config",
    category: "settings",
    action: tracked("Refresh Config", "settings", async () => {
      await restClient.post("/api/config/sync");
      addToast("success", "Config refreshed from repo");
      return "Refreshed";
    }),
  },
  {
    id: "config.skills-sync",
    label: "Sync Skills from Repo",
    category: "settings",
    action: tracked("Sync Skills", "settings", async () => {
      await restClient.post("/api/config/sync");
      addToast("success", "Skills synced from repo");
      return "Synced";
    }),
  },
  {
    id: "config.enforce-list",
    label: "List Enforcement Rules",
    category: "settings",
    action: tracked("List Enforcement Rules", "settings", async () => {
      const data = await restClient.get<any>("/api/hexflo/enforcement-rules");
      const rules = data.rules ?? [];
      addToast("success", `${rules.length} enforcement rule(s)`);
      return `${rules.length} rule(s)`;
    }),
  },

  // ── Settings / Help ──
  { id: "help.shortcuts", label: "Show Keyboard Shortcuts", category: "settings", shortcut: "Ctrl+?", action: () => setShortcutsOpen(true) },
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
