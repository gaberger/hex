import { type Component, onMount, onCleanup, createSignal, Show, Switch, Match } from 'solid-js';
import { initConnections } from '../stores/connection';
import {
  splitPane,
  closePane,
  toggleMaximize,
  focusNextPane,
  focusPrevPane,
  focusPaneByIndex,
} from '../stores/panes';
import ContextPanel from '../components/layout/ContextPanel';
import BottomBar from '../components/layout/BottomBar';
import Breadcrumbs from '../components/layout/Breadcrumbs';
import SpawnDialog from '../components/agent/SpawnDialog';
import SwarmInitDialog from '../components/swarm/SwarmInitDialog';
import CommandPalette from '../components/command/CommandPalette';
import ToastContainer from '../components/layout/ToastContainer';
import { spawnDialogOpen, setSpawnDialogOpen, commandPaletteOpen, setCommandPaletteOpen, swarmInitDialogOpen, setSwarmInitDialogOpen } from '../stores/ui';
import { startNexusHealthPoll, stopNexusHealthPoll } from '../stores/nexus-health';
import { mode, toggleMode } from '../stores/mode';
import { toggleViewMode } from '../stores/view';
import { initChatConnection, disconnectChat } from '../stores/chat';
import { startHexFloMonitor } from '../stores/hexflo-monitor';
import { route, initRouter, navigate } from '../stores/router';
import ChatView from '../components/chat/ChatView';
import HealthPane from '../components/health/HealthPane';
import DependencyGraphPane from '../components/graph/DependencyGraphPane';
import InferencePanel from '../components/fleet/InferencePanel';
import FleetView from '../components/fleet/FleetView';
import { ControlPlane, AgentFleet, ProjectDetail, ADRBrowser, ConfigPage } from '../components/views';

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
          <span class="text-sm font-semibold tracking-wide text-gray-100">
            HEX NEXUS
          </span>
          {/* Section navigation tabs */}
          <nav class="hidden md:flex items-center gap-1 ml-4">
            <button
              class="rounded-md px-3 py-1.5 text-xs font-medium transition-colors"
              classList={{
                "bg-gray-800 text-gray-100": route().page === "control-plane",
                "text-gray-500 hover:text-gray-300 hover:bg-gray-800/50": route().page !== "control-plane",
              }}
              onClick={() => navigate({ page: "control-plane" })}
            >
              Projects
            </button>
            <button
              class="rounded-md px-3 py-1.5 text-xs font-medium transition-colors"
              classList={{
                "bg-gray-800 text-gray-100": route().page === "agent-fleet",
                "text-gray-500 hover:text-gray-300 hover:bg-gray-800/50": route().page !== "agent-fleet",
              }}
              onClick={() => navigate({ page: "agent-fleet" })}
            >
              Agents
            </button>
            <button
              class="rounded-md px-3 py-1.5 text-xs font-medium transition-colors"
              classList={{
                "bg-gray-800 text-gray-100": route().page === "adrs" || route().page === "project-adr",
                "text-gray-500 hover:text-gray-300 hover:bg-gray-800/50": route().page !== "adrs" && route().page !== "project-adr",
              }}
              onClick={() => navigate({ page: "adrs" })}
            >
              ADRs
            </button>
            <button
              class="rounded-md px-3 py-1.5 text-xs font-medium transition-colors"
              classList={{
                "bg-gray-800 text-gray-100": route().page === "config",
                "text-gray-500 hover:text-gray-300 hover:bg-gray-800/50": route().page !== "config",
              }}
              onClick={() => navigate({ page: "config", section: "blueprint" })}
            >
              Config
            </button>
            <button
              class="rounded-md px-3 py-1.5 text-xs font-medium transition-colors"
              classList={{
                "bg-gray-800 text-gray-100": route().page === "inference",
                "text-gray-500 hover:text-gray-300 hover:bg-gray-800/50": route().page !== "inference",
              }}
              onClick={() => navigate({ page: "inference" })}
            >
              Inference
            </button>
          </nav>
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

      {/* Breadcrumbs */}
      <Breadcrumbs />

      {/* Main area — full width, no sidebar */}
      <div class="flex flex-1 overflow-hidden">
        {/* Center content — route-based view switching */}
        <div class="flex flex-1 flex-col overflow-hidden">
          <Switch fallback={<ControlPlane />}>
            <Match when={route().page === "control-plane"}>
              <ControlPlane />
            </Match>
            <Match when={route().page === "project-chat"}>
              <ChatView />
            </Match>
            <Match when={route().page === "project"}>
              <ProjectDetail />
            </Match>
            <Match when={route().page === "adrs" || route().page === "project-adr"}>
              <ADRBrowser />
            </Match>
            <Match when={route().page === "agent-fleet"}>
              <AgentFleet />
            </Match>
            <Match when={route().page === "project-health"}>
              <div class="flex-1 overflow-auto p-6">
                <HealthPane />
              </div>
            </Match>
            <Match when={route().page === "project-graph"}>
              <DependencyGraphPane />
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
          </Switch>
        </div>

        {/* Right panel: only on project-scoped pages */}
        <Show when={route().page.startsWith("project")}>
          <div class="hidden lg:block">
            <ContextPanel />
          </div>
        </Show>
      </div>

      {/* BottomBar */}
      <BottomBar />

      {/* Mobile bottom tabs — only shown on small screens */}
      <div class="flex md:hidden items-center justify-around border-t border-gray-800 bg-gray-900 py-2">
        <button class="flex flex-col items-center gap-0.5 text-gray-400 hover:text-gray-200 px-4 py-1"
          classList={{ "text-cyan-400": route().page === "control-plane" }}
          onClick={() => navigate({ page: "control-plane" })}
        >
          <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <rect x="3" y="3" width="7" height="7" />
            <rect x="14" y="3" width="7" height="7" />
            <rect x="3" y="14" width="7" height="7" />
            <rect x="14" y="14" width="7" height="7" />
          </svg>
          <span class="text-[10px]">Projects</span>
        </button>
        <button class="flex flex-col items-center gap-0.5 text-gray-400 hover:text-gray-200 px-4 py-1"
          classList={{ "text-cyan-400": route().page === "agent-fleet" }}
          onClick={() => navigate({ page: "agent-fleet" })}
        >
          <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" />
            <circle cx="9" cy="7" r="4" />
            <path d="M23 21v-2a4 4 0 0 0-3-3.87" />
            <path d="M16 3.13a4 4 0 0 1 0 7.75" />
          </svg>
          <span class="text-[10px]">Agents</span>
        </button>
        <button class="flex flex-col items-center gap-0.5 text-gray-400 hover:text-gray-200 px-4 py-1"
          classList={{ "text-cyan-400": route().page === "project-health" }}
          onClick={() => navigate({ page: "project-health", projectId: "current" })}
        >
          <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M22 12h-4l-3 9L9 3l-3 9H2" />
          </svg>
          <span class="text-[10px]">Health</span>
        </button>
        <button class="flex flex-col items-center gap-0.5 text-gray-400 hover:text-gray-200 px-4 py-1"
          classList={{ "text-cyan-400": route().page === "config" }}
          onClick={() => navigate({ page: "config", section: "blueprint" })}
        >
          <svg class="h-5 w-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
          <span class="text-[10px]">Config</span>
        </button>
      </div>

      {/* SpawnDialog overlay */}
      <SpawnDialog open={spawnDialogOpen()} onClose={() => setSpawnDialogOpen(false)} />
      <SwarmInitDialog open={swarmInitDialogOpen()} onClose={() => setSwarmInitDialogOpen(false)} />
      <CommandPalette open={commandPaletteOpen()} onClose={() => setCommandPaletteOpen(false)} />
      <ToastContainer />
    </div>
  );
};

export default App;
