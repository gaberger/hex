import { type Component, onMount, onCleanup, createSignal, For, Show, Switch, Match, lazy } from 'solid-js';
import { initConnectionStore, initConnections } from '../stores/connection';
import BottomBar from '../components/layout/BottomBar';
import Breadcrumbs from '../components/layout/Breadcrumbs';
import SpawnDialog from '../components/agent/SpawnDialog';
import SwarmInitDialog from '../components/swarm/SwarmInitDialog';
import CommandPalette from '../components/command/CommandPalette';
import ToastContainer from '../components/layout/ToastContainer';
import ShortcutsOverlay from '../components/layout/ShortcutsOverlay';
import { spawnDialogOpen, setSpawnDialogOpen, commandPaletteOpen, setCommandPaletteOpen, swarmInitDialogOpen, setSwarmInitDialogOpen, shortcutsOpen, setShortcutsOpen } from '../stores/ui';
import { startNexusHealthPoll, stopNexusHealthPoll } from '../stores/nexus-health';
import { mode, toggleMode } from '../stores/mode';
import { initChatConnection, disconnectChat } from '../stores/chat';
import { startHexFloMonitor } from '../stores/hexflo-monitor';
import { route, initRouter, initRouterStore, navigate, activeProjectId } from '../stores/router';
import type { Route } from '../stores/router';
import { projects, initProjectStore } from '../stores/projects';
import { initWorkplanStore } from '../stores/workplan';
import ChatView from '../components/chat/ChatView';
import { ControlPlane, ProjectDetail } from '../components/views';
import { ProjectHome } from '../components/project';

// Lazy-load views that are not on the initial render path
const AgentFleet = lazy(() => import('../components/views/AgentFleet'));
const ADRBrowser = lazy(() => import('../components/views/ADRBrowser'));
const ConfigPage = lazy(() => import('../components/views/ConfigPage'));
const FileTreeView = lazy(() => import('../components/views/FileTreeView'));
const WorkplanView = lazy(() => import('../components/views/WorkplanView'));
const AgentList = lazy(() => import('../components/project/AgentList'));
const AgentDetailView = lazy(() => import('../components/project/AgentDetail'));
const SwarmDetail = lazy(() => import('../components/project/SwarmDetail'));
const WorkPlanDetail = lazy(() => import('../components/project/WorkPlanDetail'));

// ── Sidebar nav item definitions ─────────────────────────────────────────────

interface NavItem {
  label: string;
  icon: string; // SVG path content
  page: string;
  routeFactory: (projectId: string) => Route;
}

const projectSubNav: NavItem[] = [
  {
    label: 'Overview',
    icon: '<rect x="3" y="3" width="18" height="18" rx="2" /><line x1="3" y1="9" x2="21" y2="9" /><line x1="9" y1="21" x2="9" y2="9" />',
    page: 'project',
    routeFactory: (pid) => ({ page: 'project', projectId: pid }),
  },
  {
    label: 'Agents',
    icon: '<path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" /><circle cx="9" cy="7" r="4" /><path d="M23 21v-2a4 4 0 0 0-3-3.87" /><path d="M16 3.13a4 4 0 0 1 0 7.75" />',
    page: 'project-agents',
    routeFactory: (pid) => ({ page: 'project-agents', projectId: pid }),
  },
  {
    label: 'Swarms',
    icon: '<polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />',
    page: 'project-swarms',
    routeFactory: (pid) => ({ page: 'project-swarms', projectId: pid }),
  },
  {
    label: 'ADRs',
    icon: '<path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><polyline points="14 2 14 8 20 8" />',
    page: 'project-adrs',
    routeFactory: (pid) => ({ page: 'project-adrs', projectId: pid }),
  },
  {
    label: 'WorkPlans',
    icon: '<path d="M9 5H7a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2h-2" /><rect x="9" y="3" width="6" height="4" rx="1" /><path d="M9 14l2 2 4-4" />',
    page: 'project-workplans',
    routeFactory: (pid) => ({ page: 'project-workplans', projectId: pid }),
  },
  {
    label: 'Health',
    icon: '<path d="M22 12h-4l-3 9L9 3l-3 9H2" />',
    page: 'project-health',
    routeFactory: (pid) => ({ page: 'project-health', projectId: pid }),
  },
  {
    label: 'Graph',
    icon: '<circle cx="18" cy="18" r="3" /><circle cx="6" cy="6" r="3" /><circle cx="18" cy="6" r="3" /><line x1="6" y1="9" x2="6" y2="21" /><path d="M6 12h6a3 3 0 0 1 3 3v3" />',
    page: 'project-graph',
    routeFactory: (pid) => ({ page: 'project-graph', projectId: pid }),
  },
  {
    label: 'Files',
    icon: '<path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />',
    page: 'project-files',
    routeFactory: (pid) => ({ page: 'project-files', projectId: pid }),
  },
  {
    label: 'Chat',
    icon: '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />',
    page: 'project-chat',
    routeFactory: (pid) => ({ page: 'project-chat', projectId: pid }),
  },
  {
    label: 'Config',
    icon: '<circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />',
    page: 'project-config',
    routeFactory: (pid) => ({ page: 'project-config', projectId: pid, section: 'blueprint' }),
  },
];

/** Returns true when the current route matches a given page prefix (handles detail pages). */
function isPageActive(page: string): boolean {
  const current = route().page;
  if (current === page) return true;
  // Detail pages: project-agent-detail matches project-agents, etc.
  if (page === 'project-agents' && current === 'project-agent-detail') return true;
  if (page === 'project-swarms' && (current === 'project-swarm-detail' || current === 'project-swarm-task')) return true;
  if (page === 'project-adrs' && current === 'project-adr-detail') return true;
  if (page === 'project-workplans' && current === 'project-workplan-detail') return true;
  if (page === 'project-files' && current === 'project-file') return true;
  if (page === 'project-config' && current === 'project-config') return true;
  return false;
}

const App: Component = () => {
  const [theme, setTheme] = createSignal(
    localStorage.getItem('theme') ||
    (window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark')
  );
  const [moreMenuOpen, setMoreMenuOpen] = createSignal(false);
  const [sidebarCollapsed, setSidebarCollapsed] = createSignal(false);
  const [mobileDrawerOpen, setMobileDrawerOpen] = createSignal(false);

  // Initialize reactive stores SYNCHRONOUSLY before any child renders.
  // These must run during component creation (not onMount) so child
  // components capture the real signal accessors, not the default () => []
  // stubs. See ADR-2603231000 for the full diagnosis.
  initConnectionStore();    // must be first — creates signals other stores depend on
  initProjectStore();       // depends on registeredProjects from connection
  initRouterStore();        // depends on projects from project store
  initWorkplanStore();      // independent — REST-backed, no store dependencies

  onMount(() => {
    // Start connections and monitors (uses signals created above)
    initConnections();
    startNexusHealthPoll();
    initChatConnection();
    startHexFloMonitor();
    initRouter();
    document.documentElement.setAttribute('data-theme', theme());
  });

  onCleanup(() => {
    stopNexusHealthPoll();
    disconnectChat();
  });

  const toggleTheme = () => {
    const newTheme = theme() === 'light' ? 'dark' : 'light';
    setTheme(newTheme);
    localStorage.setItem('theme', newTheme);
    document.documentElement.setAttribute('data-theme', newTheme);
  };

  // ── Keyboard shortcuts ──
  function handleKeyboard(e: KeyboardEvent) {
    const ctrl = e.ctrlKey || e.metaKey;

    // Tab (no modifier, no focused input) -- toggle mode
    if (e.key === 'Tab' && !ctrl && !e.shiftKey) {
      const active = document.activeElement;
      const isInput = active?.tagName === 'INPUT' || active?.tagName === 'TEXTAREA';
      if (!isInput) {
        e.preventDefault();
        toggleMode();
      }
    }

    // Ctrl+N -- spawn agent dialog
    if (ctrl && e.key === 'n') {
      e.preventDefault();
      setSpawnDialogOpen(true);
    }
    // Ctrl+P or Ctrl+K -- command palette
    if (ctrl && (e.key === 'p' || e.key === 'k')) {
      e.preventDefault();
      setCommandPaletteOpen(!commandPaletteOpen());
    }
    // Ctrl+? (Ctrl+Shift+/) -- shortcuts help
    if (ctrl && e.shiftKey && e.key === '?') {
      e.preventDefault();
      setShortcutsOpen(!shortcutsOpen());
    }

    // Ctrl+1-5 -- project sub-page quick navigation (only when a project is active)
    const pid = activeProjectId();
    if (ctrl && pid && !e.shiftKey) {
      const numShortcuts: Record<string, Route> = {
        '1': { page: 'project', projectId: pid },
        '2': { page: 'project-agents', projectId: pid },
        '3': { page: 'project-swarms', projectId: pid },
        '4': { page: 'project-adrs', projectId: pid },
        '5': { page: 'project-chat', projectId: pid },
      };
      const target = numShortcuts[e.key];
      if (target) {
        e.preventDefault();
        navigate(target);
      }
    }
  }

  onMount(() => {
    window.addEventListener('keydown', handleKeyboard);
  });
  onCleanup(() => {
    window.removeEventListener('keydown', handleKeyboard);
  });

  return (
    <div class="flex h-screen flex-col bg-gray-950 text-gray-100">
      {/* TopBar */}
      <header class="flex h-12 shrink-0 items-center justify-between border-b border-gray-800 bg-gray-900 px-4">
        <div class="flex items-center gap-3">
          {/* Mobile hamburger */}
          <button
            class="md:hidden rounded p-1.5 text-gray-400 hover:bg-gray-800 hover:text-gray-200 transition-colors"
            aria-label="Toggle navigation menu"
            onClick={() => setMobileDrawerOpen((v) => !v)}
          >
            <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <line x1="3" y1="6" x2="21" y2="6" /><line x1="3" y1="12" x2="21" y2="12" /><line x1="3" y1="18" x2="21" y2="18" />
            </svg>
          </button>
          <svg class="h-5 w-5" viewBox="0 0 64 64" fill="none">
            <defs>
              <linearGradient id="hex-lg" x1="0%" y1="0%" x2="100%" y2="100%">
                <stop offset="0%" stop-color="#00d4aa" />
                <stop offset="100%" stop-color="#3b82f6" />
              </linearGradient>
            </defs>
            <polygon points="32,4 58,19 58,45 32,60 6,45 6,19" fill="url(#hex-lg)" opacity=".12" stroke="url(#hex-lg)" stroke-width="2.5" />
            <polygon points="32,16 46,24 46,40 32,48 18,40 18,24" fill="url(#hex-lg)" opacity=".25" stroke="url(#hex-lg)" stroke-width="1.5" />
            <polygon points="32,27 37,30 37,34 32,37 27,34 27,30" fill="url(#hex-lg)" opacity=".8" />
          </svg>
          <button class="text-sm font-semibold tracking-wide text-gray-100 hover:text-cyan-300 transition-colors focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none rounded" onClick={() => navigate({ page: "control-plane" })}>
            HEX NEXUS
          </button>
          {/* Plan/Build mode */}
          <button
            class="flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors ml-3 focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
            classList={{
              "bg-blue-900/30 text-blue-400 hover:bg-blue-900/50": mode() === "plan",
              "bg-green-900/30 text-green-400 hover:bg-green-900/50": mode() === "build",
            }}
            onClick={toggleMode}
            title="Toggle Plan/Build mode (Tab)"
          >
            <span class="h-1.5 w-1.5 rounded-full"
              classList={{
                "bg-blue-400": mode() === "plan",
                "bg-green-400": mode() === "build",
              }}
            />
            {mode() === "plan" ? "Plan" : "Build"}
          </button>
        </div>
        <div class="flex items-center gap-3 text-[10px] text-gray-300">
          <kbd class="hidden md:inline rounded border border-gray-700 bg-gray-800 px-1.5 py-0.5 text-gray-300">Ctrl+K</kbd>
          <button
            class="rounded p-1.5 text-gray-300 hover:bg-gray-800 transition-colors focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
            aria-label="Toggle theme"
            onClick={toggleTheme}
            title={theme() === 'dark' ? 'Switch to light mode' : 'Switch to dark mode'}
          >
            {theme() === 'dark' ? (
              <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <circle cx="12" cy="12" r="5" />
                <line x1="12" y1="1" x2="12" y2="3" />
                <line x1="12" y1="21" x2="12" y2="23" />
                <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" />
                <line x1="18.36" y1="18.36" x2="19.78" y2="19.78" />
                <line x1="1" y1="12" x2="3" y2="12" />
                <line x1="21" y1="12" x2="23" y2="12" />
                <line x1="4.22" y1="19.78" x2="5.64" y2="18.36" />
                <line x1="18.36" y1="5.64" x2="19.78" y2="4.22" />
              </svg>
            ) : (
              <svg class="h-4 w-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
              </svg>
            )}
          </button>
        </div>
      </header>

      {/* Main area */}
      <div class="flex flex-1 overflow-hidden">
        {/* ── Mobile drawer backdrop ── */}
        <Show when={mobileDrawerOpen()}>
          <div class="fixed inset-0 z-30 bg-black/50 md:hidden" onClick={() => setMobileDrawerOpen(false)} />
        </Show>

        {/* ── Left sidebar (collapsible on desktop, drawer on mobile) ── */}
        <nav
          role="navigation"
          aria-label="Main navigation"
          class="flex shrink-0 flex-col border-r border-gray-800 bg-gray-950 overflow-y-auto transition-all duration-200 ease-in-out"
          classList={{
            "w-12": sidebarCollapsed(),
            "w-[272px]": !sidebarCollapsed(),
            "max-md:fixed max-md:inset-y-0 max-md:left-0 max-md:z-40 max-md:w-[272px] max-md:shadow-2xl": true,
            "max-md:-translate-x-full": !mobileDrawerOpen(),
            "max-md:translate-x-0": mobileDrawerOpen(),
          }}
        >

          {/* Control Plane link */}
          <div class="px-3 pt-3 pb-1">
            <button
              class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm font-semibold transition-colors focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
              classList={{
                "bg-gray-900/50 text-gray-100 border-l-2 border-cyan-500": route().page === "control-plane",
                "text-gray-400 hover:bg-gray-900/30 hover:text-gray-200": route().page !== "control-plane",
                "justify-center px-0": sidebarCollapsed(),
              }}
              aria-label="Control Plane"
              aria-current={route().page === "control-plane" ? "page" : undefined}
              onClick={() => { navigate({ page: "control-plane" }); setMobileDrawerOpen(false); }}
            >
              <svg class="h-4 w-4 shrink-0 text-cyan-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <rect x="3" y="3" width="7" height="7" /><rect x="14" y="3" width="7" height="7" />
                <rect x="3" y="14" width="7" height="7" /><rect x="14" y="14" width="7" height="7" />
              </svg>
              <Show when={!sidebarCollapsed()}>Control Plane</Show>
            </button>
          </div>

          {/* Projects section */}
          <div class="px-3 pb-2 pt-2">
            <Show when={!sidebarCollapsed()}>
              <div class="text-[10px] uppercase tracking-wider text-gray-500 font-semibold px-3 mb-2">Projects</div>
            </Show>
            <For each={projects()}>
              {(p) => (
                <button
                  class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm transition-colors mb-0.5 focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
                  classList={{
                    "border-l-2 border-cyan-500 bg-gray-900/50 text-gray-100 font-medium": activeProjectId() === p.id,
                    "text-gray-400 hover:text-gray-200 hover:bg-gray-900/30": activeProjectId() !== p.id,
                    "justify-center px-0": sidebarCollapsed(),
                  }}
                  aria-label={sidebarCollapsed() ? p.name : undefined}
                  aria-current={activeProjectId() === p.id ? "page" : undefined}
                  onClick={() => { navigate({ page: "project", projectId: p.id }); setMobileDrawerOpen(false); }}
                >
                  <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
                    classList={{ "text-cyan-400": activeProjectId() === p.id, "text-gray-600": activeProjectId() !== p.id }}>
                    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                  </svg>
                  <Show when={!sidebarCollapsed()}><span class="truncate">{p.name}</span></Show>
                </button>
              )}
            </For>
            <Show when={projects().length === 0 && !sidebarCollapsed()}>
              <p class="px-3 py-2 text-xs text-gray-600">No projects registered</p>
            </Show>
          </div>

          {/* Project sub-nav -- only visible when a project is active */}
          <Show when={activeProjectId()}>
            <div class="px-3 pb-2 border-t border-gray-800 pt-2">
              <Show when={!sidebarCollapsed()}>
                <div class="text-[10px] uppercase tracking-wider text-gray-500 font-semibold px-3 mb-2">Project</div>
              </Show>
              <For each={projectSubNav}>
                {(item) => (
                  <button
                    class="flex w-full items-center gap-2 rounded-lg px-3 py-1.5 text-[13px] transition-colors mb-0.5 focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
                    classList={{
                      "border-l-2 border-cyan-500 bg-gray-900/50 text-gray-100": isPageActive(item.page),
                      "text-gray-400 hover:text-gray-200 hover:bg-gray-900/30": !isPageActive(item.page),
                      "justify-center px-0": sidebarCollapsed(),
                    }}
                    aria-label={sidebarCollapsed() ? item.label : undefined}
                    aria-current={isPageActive(item.page) ? "page" : undefined}
                    onClick={() => { navigate(item.routeFactory(activeProjectId())); setMobileDrawerOpen(false); }}
                  >
                    <svg class="h-3.5 w-3.5 shrink-0 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
                      classList={{ "text-cyan-400": isPageActive(item.page) }}
                      innerHTML={item.icon}
                    />
                    <Show when={!sidebarCollapsed()}>{item.label}</Show>
                  </button>
                )}
              </For>
            </div>
          </Show>

          {/* Global section -- pinned to bottom */}
          <div class="px-3 pb-3 border-t border-gray-800 pt-2 mt-auto">
            <Show when={!sidebarCollapsed()}>
              <div class="text-[10px] uppercase tracking-wider text-gray-500 font-semibold px-3 mb-2">Global</div>
            </Show>
            {/* Inference */}
            <button
              class="flex w-full items-center gap-2 rounded-lg px-3 py-1.5 text-[13px] transition-colors mb-0.5 focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
              classList={{
                "border-l-2 border-cyan-500 bg-gray-900/50 text-gray-100": route().page === "inference",
                "text-gray-400 hover:text-gray-200 hover:bg-gray-900/30": route().page !== "inference",
                "justify-center px-0": sidebarCollapsed(),
              }}
              aria-label={sidebarCollapsed() ? "Inference" : undefined}
              aria-current={route().page === "inference" ? "page" : undefined}
              onClick={() => { navigate({ page: "inference" }); setMobileDrawerOpen(false); }}
            >
              <svg class="h-3.5 w-3.5 shrink-0 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
                classList={{ "text-cyan-400": route().page === "inference" }}>
                <rect x="2" y="2" width="20" height="8" rx="2" /><rect x="2" y="14" width="20" height="8" rx="2" />
                <line x1="6" y1="6" x2="6.01" y2="6" /><line x1="6" y1="18" x2="6.01" y2="18" />
              </svg>
              <Show when={!sidebarCollapsed()}>Inference</Show>
            </button>
            {/* Fleet Nodes */}
            <button
              class="flex w-full items-center gap-2 rounded-lg px-3 py-1.5 text-[13px] transition-colors mb-0.5 focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
              classList={{
                "border-l-2 border-cyan-500 bg-gray-900/50 text-gray-100": route().page === "fleet",
                "text-gray-400 hover:text-gray-200 hover:bg-gray-900/30": route().page !== "fleet",
                "justify-center px-0": sidebarCollapsed(),
              }}
              aria-label={sidebarCollapsed() ? "Fleet Nodes" : undefined}
              aria-current={route().page === "fleet" ? "page" : undefined}
              onClick={() => { navigate({ page: "fleet" }); setMobileDrawerOpen(false); }}
            >
              <svg class="h-3.5 w-3.5 shrink-0 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
                classList={{ "text-cyan-400": route().page === "fleet" }}>
                <rect x="2" y="3" width="20" height="14" rx="2" /><line x1="8" y1="21" x2="16" y2="21" />
                <line x1="12" y1="17" x2="12" y2="21" />
              </svg>
              <Show when={!sidebarCollapsed()}>Fleet Nodes</Show>
            </button>

            {/* Collapse toggle (desktop only) */}
            <button
              class="hidden md:flex w-full items-center gap-2 rounded-lg px-3 py-1.5 text-[13px] text-gray-500 hover:text-gray-300 hover:bg-gray-900/30 transition-colors mt-2 focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
              classList={{ "justify-center px-0": sidebarCollapsed() }}
              aria-label={sidebarCollapsed() ? "Expand sidebar" : "Collapse sidebar"}
              onClick={() => setSidebarCollapsed((v) => !v)}
            >
              <svg class="h-3.5 w-3.5 shrink-0 transition-transform duration-200" classList={{ "rotate-180": sidebarCollapsed() }} viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <polyline points="15 18 9 12 15 6" />
              </svg>
              <Show when={!sidebarCollapsed()}>Collapse</Show>
            </button>
          </div>
        </nav>

        {/* ── Center content -- route-based view switching ── */}
        <div class="flex flex-1 flex-col overflow-hidden">
          <Breadcrumbs />
          <Switch fallback={<ControlPlane />}>
            <Match when={route().page === "control-plane"}><ControlPlane /></Match>
            <Match when={route().page === "project"}><ProjectHome /></Match>
            <Match when={route().page === "project-agents"}><AgentList /></Match>
            <Match when={route().page === "project-agent-detail"}><AgentDetailView /></Match>
            <Match when={route().page === "project-swarms"}><ProjectHome /></Match>
            <Match when={route().page === "project-swarm-detail"}><SwarmDetail /></Match>
            <Match when={route().page === "project-swarm-task"}><SwarmDetail /></Match>
            <Match when={route().page === "project-adrs"}><ADRBrowser /></Match>
            <Match when={route().page === "project-adr-detail"}><ADRBrowser /></Match>
            <Match when={route().page === "project-workplans"}><WorkplanView /></Match>
            <Match when={route().page === "project-workplan-detail"}><WorkPlanDetail /></Match>
            <Match when={route().page === "project-health"}><ProjectDetail /></Match>
            <Match when={route().page === "project-graph"}><ProjectDetail /></Match>
            <Match when={route().page === "project-files"}><FileTreeView /></Match>
            <Match when={route().page === "project-file"}><FileTreeView /></Match>
            <Match when={route().page === "project-chat"}><ChatView /></Match>
            <Match when={route().page === "project-config"}><ConfigPage /></Match>
            <Match when={route().page === "inference"}><ControlPlane /></Match>
            <Match when={route().page === "fleet"}><ControlPlane /></Match>
          </Switch>
          {/* BottomBar -- inside center content so it doesn't span under sidebar */}
          <BottomBar />
        </div>

      </div>

      {/* Mobile bottom tabs -- only shown on small screens */}
      <div class="relative flex md:hidden items-center justify-around border-t border-gray-800 bg-gray-900 py-2">
        {/* More menu slide-up overlay */}
        <Show when={moreMenuOpen()}>
          <div class="absolute bottom-full left-0 right-0 z-50 border-t border-gray-700 bg-gray-900 shadow-2xl transition-transform duration-200 ease-out animate-slide-up">
            <div class="grid grid-cols-2 gap-1 p-3">
              {/* Global items */}
              <button
                class="flex items-center gap-2 rounded-lg px-3 py-2.5 text-sm text-gray-300 hover:bg-gray-800 hover:text-gray-100 transition-colors focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
                classList={{ "bg-gray-800 text-cyan-400": route().page === "inference" }}
                onClick={() => { navigate({ page: "inference" }); setMoreMenuOpen(false); }}
              >
                <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <rect x="2" y="2" width="20" height="8" rx="2" /><rect x="2" y="14" width="20" height="8" rx="2" />
                </svg>
                Inference
              </button>
              <button
                class="flex items-center gap-2 rounded-lg px-3 py-2.5 text-sm text-gray-300 hover:bg-gray-800 hover:text-gray-100 transition-colors focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
                classList={{ "bg-gray-800 text-cyan-400": route().page === "fleet" }}
                onClick={() => { navigate({ page: "fleet" }); setMoreMenuOpen(false); }}
              >
                <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <rect x="2" y="3" width="20" height="14" rx="2" /><line x1="8" y1="21" x2="16" y2="21" />
                  <line x1="12" y1="17" x2="12" y2="21" />
                </svg>
                Fleet Nodes
              </button>
              {/* Project-scoped items */}
              <Show when={activeProjectId()}>
                <button
                  class="flex items-center gap-2 rounded-lg px-3 py-2.5 text-sm text-gray-300 hover:bg-gray-800 hover:text-gray-100 transition-colors focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
                  classList={{ "bg-gray-800 text-cyan-400": isPageActive("project-adrs") }}
                  onClick={() => { navigate({ page: "project-adrs", projectId: activeProjectId() }); setMoreMenuOpen(false); }}
                >
                  <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                    <polyline points="14 2 14 8 20 8" />
                  </svg>
                  ADRs
                </button>
                <button
                  class="flex items-center gap-2 rounded-lg px-3 py-2.5 text-sm text-gray-300 hover:bg-gray-800 hover:text-gray-100 transition-colors focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
                  classList={{ "bg-gray-800 text-cyan-400": isPageActive("project-workplans") }}
                  onClick={() => { navigate({ page: "project-workplans", projectId: activeProjectId() }); setMoreMenuOpen(false); }}
                >
                  <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M9 5H7a2 2 0 0 0-2 2v12a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V7a2 2 0 0 0-2-2h-2" />
                    <rect x="9" y="3" width="6" height="4" rx="1" />
                    <path d="M9 14l2 2 4-4" />
                  </svg>
                  WorkPlans
                </button>
                <button
                  class="flex items-center gap-2 rounded-lg px-3 py-2.5 text-sm text-gray-300 hover:bg-gray-800 hover:text-gray-100 transition-colors focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
                  classList={{ "bg-gray-800 text-cyan-400": isPageActive("project-config") }}
                  onClick={() => { navigate({ page: "project-config", projectId: activeProjectId(), section: "blueprint" }); setMoreMenuOpen(false); }}
                >
                  <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <circle cx="12" cy="12" r="3" />
                    <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
                  </svg>
                  Config
                </button>
                <button
                  class="flex items-center gap-2 rounded-lg px-3 py-2.5 text-sm text-gray-300 hover:bg-gray-800 hover:text-gray-100 transition-colors focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
                  classList={{ "bg-gray-800 text-cyan-400": isPageActive("project-health") }}
                  onClick={() => { navigate({ page: "project-health", projectId: activeProjectId() }); setMoreMenuOpen(false); }}
                >
                  <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M22 12h-4l-3 9L9 3l-3 9H2" />
                  </svg>
                  Health
                </button>
                <button
                  class="flex items-center gap-2 rounded-lg px-3 py-2.5 text-sm text-gray-300 hover:bg-gray-800 hover:text-gray-100 transition-colors focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none"
                  classList={{ "bg-gray-800 text-cyan-400": isPageActive("project-files") }}
                  onClick={() => { navigate({ page: "project-files", projectId: activeProjectId() }); setMoreMenuOpen(false); }}
                >
                  <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                  </svg>
                  Files
                </button>
              </Show>
            </div>
          </div>
          {/* Backdrop to close */}
          <div class="fixed inset-0 z-40" onClick={() => setMoreMenuOpen(false)} />
        </Show>

        {/* Tab 1: Projects (Control Plane) */}
        <button class="flex flex-col items-center gap-0.5 text-gray-400 hover:text-gray-200 px-3 py-1 focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none rounded"
          classList={{ "text-cyan-400": route().page === "control-plane" }}
          aria-label="Projects"
          onClick={() => { navigate({ page: "control-plane" }); setMoreMenuOpen(false); }}
        >
          <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <rect x="3" y="3" width="7" height="7" /><rect x="14" y="3" width="7" height="7" />
            <rect x="3" y="14" width="7" height="7" /><rect x="14" y="14" width="7" height="7" />
          </svg>
          <span class="text-[10px]">Projects</span>
        </button>
        {/* Tab 2: Agents */}
        <button class="flex flex-col items-center gap-0.5 text-gray-400 hover:text-gray-200 px-3 py-1 focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none rounded"
          classList={{ "text-cyan-400": isPageActive("project-agents") }}
          aria-label="Agents"
          onClick={() => {
            if (activeProjectId()) navigate({ page: "project-agents", projectId: activeProjectId() });
            setMoreMenuOpen(false);
          }}
        >
          <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" /><circle cx="9" cy="7" r="4" />
          </svg>
          <span class="text-[10px]">Agents</span>
        </button>
        {/* Tab 3: Swarms */}
        <button class="flex flex-col items-center gap-0.5 text-gray-400 hover:text-gray-200 px-3 py-1 focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none rounded"
          classList={{ "text-cyan-400": isPageActive("project-swarms") }}
          aria-label="Swarms"
          onClick={() => {
            if (activeProjectId()) navigate({ page: "project-swarms", projectId: activeProjectId() });
            setMoreMenuOpen(false);
          }}
        >
          <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
          </svg>
          <span class="text-[10px]">Swarms</span>
        </button>
        {/* Tab 4: Chat */}
        <button class="flex flex-col items-center gap-0.5 text-gray-400 hover:text-gray-200 px-3 py-1 focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none rounded"
          classList={{ "text-cyan-400": isPageActive("project-chat") }}
          aria-label="Chat"
          onClick={() => {
            if (activeProjectId()) navigate({ page: "project-chat", projectId: activeProjectId() });
            setMoreMenuOpen(false);
          }}
        >
          <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
          </svg>
          <span class="text-[10px]">Chat</span>
        </button>
        {/* Tab 5: More */}
        <button class="flex flex-col items-center gap-0.5 text-gray-400 hover:text-gray-200 px-3 py-1 focus-visible:ring-2 focus-visible:ring-cyan-500/40 focus-visible:outline-none rounded"
          classList={{ "text-cyan-400": moreMenuOpen() }}
          aria-label="More options"
          onClick={() => setMoreMenuOpen(!moreMenuOpen())}
        >
          <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <circle cx="12" cy="5" r="1.5" fill="currentColor" /><circle cx="12" cy="12" r="1.5" fill="currentColor" /><circle cx="12" cy="19" r="1.5" fill="currentColor" />
          </svg>
          <span class="text-[10px]">More</span>
        </button>
      </div>

      {/* Modal overlays */}
      <SpawnDialog open={spawnDialogOpen()} onClose={() => setSpawnDialogOpen(false)} />
      <SwarmInitDialog open={swarmInitDialogOpen()} onClose={() => setSwarmInitDialogOpen(false)} />
      <CommandPalette open={commandPaletteOpen()} onClose={() => setCommandPaletteOpen(false)} />
      <ShortcutsOverlay open={shortcutsOpen()} onClose={() => setShortcutsOpen(false)} />
      <ToastContainer />
    </div>
  );
};

export default App;
