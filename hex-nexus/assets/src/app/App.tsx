import { type Component, onMount, onCleanup, createSignal } from 'solid-js';
import { initConnections } from '../stores/connection';
import {
  splitPane,
  closePane,
  toggleMaximize,
  focusNextPane,
  focusPrevPane,
  replaceActivePane,
} from '../stores/panes';
import Sidebar from '../components/layout/Sidebar';
import RightPanel from '../components/layout/RightPanel';
import BottomBar from '../components/layout/BottomBar';
import PaneManager from '../components/panes/PaneManager';
import SpawnDialog from '../components/agent/SpawnDialog';
import CommandPalette from '../components/command/CommandPalette';
import { spawnDialogOpen, setSpawnDialogOpen, commandPaletteOpen, setCommandPaletteOpen } from '../stores/ui';

const App: Component = () => {
  const [theme, setTheme] = createSignal(
    localStorage.getItem('theme') || 
    (window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark')
  );

  onMount(() => {
    initConnections();
    // Apply saved theme on load
    document.documentElement.setAttribute('data-theme', theme());
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
    // Ctrl+N — spawn agent dialog
    if (ctrl && e.key === 'n') {
      e.preventDefault();
      setSpawnDialogOpen(true);
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
        </div>
        <div class="flex items-center gap-3 text-[10px] text-gray-300">
          <span class="hidden sm:inline">
            <kbd class="rounded border border-gray-700 bg-gray-800 px-1 py-0.5 text-gray-300">Ctrl+\</kbd> split
            <kbd class="ml-2 rounded border border-gray-700 bg-gray-800 px-1 py-0.5 text-gray-300">Ctrl+W</kbd> close
          </span>
          <kbd class="rounded border border-gray-700 bg-gray-800 px-1.5 py-0.5 text-gray-300">Ctrl+P</kbd>
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

      {/* Main 3-column area */}
      <div class="flex flex-1 overflow-hidden">
        <Sidebar />
        <PaneManager />
        <RightPanel />
      </div>

      {/* BottomBar */}
      <BottomBar />

      {/* SpawnDialog overlay */}
      <SpawnDialog open={spawnDialogOpen()} onClose={() => setSpawnDialogOpen(false)} />
      <CommandPalette open={commandPaletteOpen()} onClose={() => setCommandPaletteOpen(false)} />
    </div>
  );
};

export default App;
