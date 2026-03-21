import { type Component, onMount, onCleanup, createSignal, For, Show, Switch, Match, lazy } from 'solid-js';
import { initConnections } from '../stores/connection';
import {
  splitPane,
  closePane,
  toggleMaximize,
  focusNextPane,
  focusPrevPane,
  focusPaneByIndex,
} from '../stores/panes';
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
import { toggleViewMode } from '../stores/view';
import { initChatConnection, disconnectChat } from '../stores/chat';
import { startHexFloMonitor } from '../stores/hexflo-monitor';
import { route, initRouter, navigate, activeProjectId } from '../stores/router';
import { projects } from '../stores/projects';
import ChatView from '../components/chat/ChatView';
import HealthPane from '../components/health/HealthPane';
import DependencyGraphPane from '../components/graph/DependencyGraphPane';
import InferencePanel from '../components/fleet/InferencePanel';
import FleetView from '../components/fleet/FleetView';
import { ControlPlane, ProjectDetail } from '../components/views';

// Lazy-load views that are not on the initial render path (T25 perf audit)
const AgentFleet = lazy(() => import('../components/views/AgentFleet'));
const ADRBrowser = lazy(() => import('../components/views/ADRBrowser'));
const ConfigPage = lazy(() => import('../components/views/ConfigPage'));
const FileTreeView = lazy(() => import('../components/views/FileTreeView'));

const App: Component = () => {
  const [theme, setTheme] = createSignal(
    localStorage.getItem('theme') || 
    (window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark')
  );

  onMount(() => {
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

  // ── Keyboard shortcuts (tiling WM style) ──
  function handleKeyboard(e: KeyboardEvent) {
    const ctrl = e.ctrlKey || e.metaKey;

    // Tab (no modifier, no focused input) — toggle mode
    if (e.key === 'Tab' && !ctrl && !e.shiftKey) {
      const active = document.activeElement;
      const isInput = active?.tagName === 'INPUT' || active?.tagName === 'TEXTAREA';
      if (!isInput) {
        e.preventDefault();
        toggleMode();
      }
    }

    // Ctrl+\ — split horizontal
    if (ctrl && e.key === '\\') {
      e.preventDefault();
      splitPane('horizontal');
    }
    // Ctrl+- — split vertical
    if (ctrl && e.key === '-') {
      e.preventDefault();
      splitPane('vertical');
    }
    // Ctrl+W — close active pane
    if (ctrl && e.key === 'w') {
      e.preventDefault();
      closePane();
    }
    // Ctrl+Shift+Enter — toggle maximize
    if (ctrl && e.shiftKey && e.key === 'Enter') {
      e.preventDefault();
      toggleMaximize();
    }
    // Ctrl+] — focus next pane
    if (ctrl && e.key === ']') {
      e.preventDefault();
      focusNextPane();
    }
    // Ctrl+[ — focus prev pane
    if (ctrl && e.key === '[') {
      e.preventDefault();
      focusPrevPane();
    }
    // Ctrl+[1-9] — focus pane by number
    if (ctrl && e.key >= '1' && e.key <= '9') {
      e.preventDefault();
      focusPaneByIndex(parseInt(e.key, 10));
    }
    // Ctrl+N — spawn agent dialog
    if (ctrl && e.key === 'n') {
      e.preventDefault();
      setSpawnDialogOpen(true);
    }
    // Ctrl+Shift+C — toggle chat/panes view
    if (ctrl && e.shiftKey && e.key === 'C') {
      e.preventDefault();
      toggleViewMode();
    }
    // Ctrl+P — command palette
    if (ctrl && e.key === 'p') {
      e.preventDefault();
      setCommandPaletteOpen(!commandPaletteOpen());
    }
    // Ctrl+? (Ctrl+Shift+/) — shortcuts help
    if (ctrl && e.shiftKey && e.key === '?') {
      e.preventDefault();
      setShortcutsOpen(!shortcutsOpen());
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
          <button class="text-sm font-semibold tracking-wide text-gray-100 hover:text-cyan-300 transition-colors" onClick={() => navigate({ page: "control-plane" })}>
            HEX NEXUS
          </button>
          {/* Plan/Build mode */}
          <button
            class="flex items-center gap-1.5 rounded-md px-2.5 py-1 text-xs font-medium transition-colors ml-3"
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
          <span class="hidden lg:inline">
            <kbd class="rounded border border-gray-700 bg-gray-800 px-1 py-0.5 text-gray-300">Ctrl+\</kbd> split
            <kbd class="ml-2 rounded border border-gray-700 bg-gray-800 px-1 py-0.5 text-gray-300">Ctrl+W</kbd> close
          </span>
          <kbd class="hidden md:inline rounded border border-gray-700 bg-gray-800 px-1.5 py-0.5 text-gray-300">Ctrl+P</kbd>
          <button
            class="rounded p-1.5 text-gray-300 hover:bg-gray-800 transition-colors"
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
        {/* Permanent left nav bar — all navigation lives here */}
        <nav class="hidden md:flex w-52 shrink-0 flex-col border-r border-gray-800 bg-gray-900 overflow-y-auto">
          {/* Control Plane / All Projects */}
          <div class="px-3 pt-3 pb-1">
            <button
              class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm font-semibold transition-colors"
              classList={{
                "bg-gray-800 text-gray-100": route().page === "control-plane",
                "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200": route().page !== "control-plane",
              }}
              onClick={() => navigate({ page: "control-plane" })}
            >
              <svg class="h-4 w-4 text-cyan-400" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <rect x="3" y="3" width="7" height="7" /><rect x="14" y="3" width="7" height="7" />
                <rect x="3" y="14" width="7" height="7" /><rect x="14" y="14" width="7" height="7" />
              </svg>
              All Projects
            </button>
          </div>

          {/* Project list */}
          <div class="px-3 pb-2">
            <div class="text-[10px] font-bold uppercase tracking-wider text-gray-600 px-3 mb-2">Projects</div>
            <For each={projects()}>
              {(p) => (
                <button
                  class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm transition-colors mb-0.5"
                  classList={{
                    "bg-cyan-900/20 text-cyan-300 font-medium": activeProjectId() === p.id,
                    "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200": activeProjectId() !== p.id,
                  }}
                  onClick={() => navigate({ page: "project", projectId: p.id })}
                >
                  <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"
                    classList={{ "text-cyan-400": activeProjectId() === p.id, "text-gray-600": activeProjectId() !== p.id }}>
                    <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                  </svg>
                  <span class="truncate">{p.name}</span>
                </button>
              )}
            </For>
            <Show when={projects().length === 0}>
              <p class="px-3 py-2 text-xs text-gray-600">No projects</p>
            </Show>
          </div>

          {/* Project-scoped nav — only visible when a project is active */}
          <Show when={activeProjectId()}>
            <div class="px-3 pb-2 border-t border-gray-800 pt-2">
              <div class="text-[10px] font-bold uppercase tracking-wider text-gray-600 px-3 mb-2">Project</div>
              {/* Config (project-scoped) */}
              <button
                class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm transition-colors mb-0.5"
                classList={{
                  "bg-gray-800 text-gray-100": route().page === "config" && !!(route() as any).projectId,
                  "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200": route().page !== "config" || !(route() as any).projectId,
                }}
                onClick={() => navigate({ page: "config", section: "blueprint", projectId: activeProjectId() })}
              >
                <svg class="h-4 w-4 shrink-0 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <circle cx="12" cy="12" r="3" />
                  <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
                </svg>
                Config
              </button>
              {/* ADRs */}
              <button
                class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm transition-colors mb-0.5"
                classList={{
                  "bg-gray-800 text-gray-100": route().page === "adrs" || route().page === "project-adr",
                  "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200": route().page !== "adrs" && route().page !== "project-adr",
                }}
                onClick={() => navigate({ page: "adrs", projectId: activeProjectId() })}
              >
                <svg class="h-4 w-4 shrink-0 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                  <polyline points="14 2 14 8 20 8" />
                </svg>
                ADRs
              </button>
              {/* Files */}
              <button
                class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm transition-colors mb-0.5"
                classList={{
                  "bg-gray-800 text-gray-100": route().page === "file-tree",
                  "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200": route().page !== "file-tree",
                }}
                onClick={() => navigate({ page: "file-tree", projectId: activeProjectId() })}
              >
                <svg class="h-4 w-4 shrink-0 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
                </svg>
                Files
              </button>
              {/* Chat */}
              <button
                class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm transition-colors mb-0.5"
                classList={{
                  "bg-gray-800 text-gray-100": route().page === "project-chat",
                  "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200": route().page !== "project-chat",
                }}
                onClick={() => navigate({ page: "project-chat", projectId: activeProjectId() })}
              >
                <svg class="h-4 w-4 shrink-0 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
                </svg>
                Chat
              </button>
              {/* Health */}
              <button
                class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm transition-colors mb-0.5"
                classList={{
                  "bg-gray-800 text-gray-100": route().page === "project-health",
                  "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200": route().page !== "project-health",
                }}
                onClick={() => navigate({ page: "project-health", projectId: activeProjectId() })}
              >
                <svg class="h-4 w-4 shrink-0 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <path d="M22 12h-4l-3 9L9 3l-3 9H2" />
                </svg>
                Health
              </button>
              {/* Dependencies */}
              <button
                class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm transition-colors mb-0.5"
                classList={{
                  "bg-gray-800 text-gray-100": route().page === "project-graph",
                  "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200": route().page !== "project-graph",
                }}
                onClick={() => navigate({ page: "project-graph", projectId: activeProjectId() })}
              >
                <svg class="h-4 w-4 shrink-0 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <circle cx="18" cy="18" r="3" /><circle cx="6" cy="6" r="3" /><circle cx="18" cy="6" r="3" />
                  <line x1="6" y1="9" x2="6" y2="21" /><path d="M6 12h6a3 3 0 0 1 3 3v3" />
                </svg>
                Dependencies
              </button>
            </div>
          </Show>

          {/* Global nav — always visible */}
          <div class="px-3 pb-3 border-t border-gray-800 pt-2 mt-auto">
            <div class="text-[10px] font-bold uppercase tracking-wider text-gray-600 px-3 mb-2">System</div>
            {/* Agents */}
            <button
              class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm transition-colors mb-0.5"
              classList={{
                "bg-gray-800 text-gray-100": route().page === "agent-fleet",
                "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200": route().page !== "agent-fleet",
              }}
              onClick={() => navigate({ page: "agent-fleet" })}
            >
              <svg class="h-4 w-4 shrink-0 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" /><circle cx="9" cy="7" r="4" />
                <path d="M23 21v-2a4 4 0 0 0-3-3.87" /><path d="M16 3.13a4 4 0 0 1 0 7.75" />
              </svg>
              Agents
            </button>
            {/* Inference */}
            <button
              class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm transition-colors mb-0.5"
              classList={{
                "bg-gray-800 text-gray-100": route().page === "inference",
                "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200": route().page !== "inference",
              }}
              onClick={() => navigate({ page: "inference" })}
            >
              <svg class="h-4 w-4 shrink-0 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <rect x="2" y="2" width="20" height="8" rx="2" /><rect x="2" y="14" width="20" height="8" rx="2" />
                <line x1="6" y1="6" x2="6.01" y2="6" /><line x1="6" y1="18" x2="6.01" y2="18" />
              </svg>
              Inference
            </button>
            {/* Fleet */}
            <button
              class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm transition-colors mb-0.5"
              classList={{
                "bg-gray-800 text-gray-100": route().page === "fleet-nodes",
                "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200": route().page !== "fleet-nodes",
              }}
              onClick={() => navigate({ page: "fleet-nodes" })}
            >
              <svg class="h-4 w-4 shrink-0 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <rect x="2" y="3" width="20" height="14" rx="2" /><line x1="8" y1="21" x2="16" y2="21" />
                <line x1="12" y1="17" x2="12" y2="21" />
              </svg>
              Fleet Nodes
            </button>
            {/* Global Config */}
            <button
              class="flex w-full items-center gap-2 rounded-lg px-3 py-2 text-sm transition-colors mb-0.5"
              classList={{
                "bg-gray-800 text-gray-100": route().page === "config" && !(route() as any).projectId,
                "text-gray-400 hover:bg-gray-800/50 hover:text-gray-200": route().page !== "config" || !!(route() as any).projectId,
              }}
              onClick={() => navigate({ page: "config", section: "blueprint" })}
            >
              <svg class="h-4 w-4 shrink-0 text-gray-500" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <circle cx="12" cy="12" r="3" />
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
              </svg>
              Global Config
            </button>
          </div>
        </nav>

        {/* Center content — route-based view switching */}
        <div class="flex flex-1 flex-col overflow-hidden">
          <Breadcrumbs />
          <Switch fallback={<ControlPlane />}>
            <Match when={route().page === "control-plane"}>
              <ControlPlane />
            </Match>
            <Match when={route().page === "project"}>
              <ProjectDetail />
            </Match>
            <Match when={route().page === "project-chat"}>
              <ChatView />
            </Match>
            <Match when={(route().page === "adrs" || route().page === "project-adr") && (route() as any).projectId}>
              <ADRBrowser />
            </Match>
            <Match when={route().page === "project-health"}>
              <div class="flex-1 overflow-auto p-6">
                <HealthPane />
              </div>
            </Match>
            <Match when={route().page === "project-graph"}>
              <DependencyGraphPane />
            </Match>
            <Match when={route().page === "adrs" && !(route() as any).projectId}>
              <ADRBrowser />
            </Match>
            <Match when={route().page === "agent-fleet"}>
              <AgentFleet />
            </Match>
            <Match when={route().page === "config"}>
              <ConfigPage />
            </Match>
            <Match when={route().page === "inference"}>
              <div class="flex-1 overflow-auto">
                <InferencePanel />
              </div>
            </Match>
            <Match when={route().page === "fleet-nodes"}>
              <div class="flex-1 overflow-auto">
                <FleetView />
              </div>
            </Match>
            <Match when={route().page === "file-tree"}>
              <FileTreeView />
            </Match>
          </Switch>
          {/* BottomBar — inside center content so it doesn't span under sidebar */}
          <BottomBar />
        </div>

      </div>

      {/* Mobile bottom tabs — only shown on small screens */}
      <div class="flex md:hidden items-center justify-around border-t border-gray-800 bg-gray-900 py-2">
        <button class="flex flex-col items-center gap-0.5 text-gray-400 hover:text-gray-200 px-3 py-1"
          classList={{ "text-cyan-400": route().page === "control-plane" }}
          onClick={() => navigate({ page: "control-plane" })}
        >
          <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <rect x="3" y="3" width="7" height="7" /><rect x="14" y="3" width="7" height="7" />
            <rect x="3" y="14" width="7" height="7" /><rect x="14" y="14" width="7" height="7" />
          </svg>
          <span class="text-[10px]">Projects</span>
        </button>
        <button class="flex flex-col items-center gap-0.5 text-gray-400 hover:text-gray-200 px-3 py-1"
          classList={{ "text-cyan-400": route().page === "agent-fleet" }}
          onClick={() => navigate({ page: "agent-fleet" })}
        >
          <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" /><circle cx="9" cy="7" r="4" />
          </svg>
          <span class="text-[10px]">Agents</span>
        </button>
        <button class="flex flex-col items-center gap-0.5 text-gray-400 hover:text-gray-200 px-3 py-1"
          classList={{ "text-cyan-400": route().page === "inference" }}
          onClick={() => navigate({ page: "inference" })}
        >
          <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <rect x="2" y="2" width="20" height="8" rx="2" /><rect x="2" y="14" width="20" height="8" rx="2" />
          </svg>
          <span class="text-[10px]">Inference</span>
        </button>
      </div>

      {/* SpawnDialog overlay */}
      <SpawnDialog open={spawnDialogOpen()} onClose={() => setSpawnDialogOpen(false)} />
      <SwarmInitDialog open={swarmInitDialogOpen()} onClose={() => setSwarmInitDialogOpen(false)} />
      <CommandPalette open={commandPaletteOpen()} onClose={() => setCommandPaletteOpen(false)} />
      <ShortcutsOverlay open={shortcutsOpen()} onClose={() => setShortcutsOpen(false)} />
      <ToastContainer />
    </div>
  );
};

export default App;
