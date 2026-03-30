/**
 * ConnectionStatusBanner.tsx — Shows SpacetimeDB / nexus connectivity status.
 *
 * Three states derived from the nexus-health store:
 *   connected   — SpacetimeDB live (green, hidden by default)
 *   fallback    — nexus online but SpacetimeDB unavailable (amber)
 *   unreachable — nexus itself unreachable (red)
 *
 * Dismissed per-state; auto-reappears if state changes.
 */
import { type Component, createSignal, createEffect } from 'solid-js';
import { nexusStatus } from '../../stores/nexus-health';

type BannerState = 'connected' | 'fallback' | 'unreachable';

function deriveState(): BannerState {
  const s = nexusStatus();
  if (!s.online) return 'unreachable';
  if (s.spacetimedb) return 'connected';
  return 'fallback';
}

const ConnectionStatusBanner: Component = () => {
  // Track which state was last dismissed so we can re-show on transitions.
  const [dismissedState, setDismissedState] = createSignal<BannerState | null>(null);

  // When state changes, clear the dismiss so the new banner is visible.
  createEffect(() => {
    const current = deriveState();
    if (dismissedState() !== null && dismissedState() !== current) {
      setDismissedState(null);
    }
  });

  const bannerState = () => deriveState();
  const visible = () => bannerState() !== 'connected' && dismissedState() !== bannerState();

  const dismiss = () => setDismissedState(bannerState());

  const nexusUrl = () => {
    const port = nexusStatus().port || 5555;
    return `http://localhost:${port}`;
  };

  return (
    <>
      {visible() && bannerState() === 'fallback' && (
        <div class="flex items-center justify-between py-1.5 px-4 bg-amber-900/80 text-amber-200 border-b border-amber-700/40 text-xs font-medium">
          <span>
            ⚠ SpacetimeDB unavailable — running in SQLite fallback mode. Some features limited.
          </span>
          <button
            class="ml-4 shrink-0 rounded p-0.5 hover:bg-amber-800/60 transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-amber-400"
            aria-label="Dismiss"
            onClick={dismiss}
          >
            <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
              <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        </div>
      )}
      {visible() && bannerState() === 'unreachable' && (
        <div class="flex items-center justify-between py-1.5 px-4 bg-red-900/80 text-red-200 border-b border-red-700/40 text-xs font-medium">
          <span>
            ✕ Cannot reach hex-nexus at {nexusUrl()}. Run <code class="font-mono bg-red-800/50 px-1 rounded">hex nexus start</code> to connect.
          </span>
          <button
            class="ml-4 shrink-0 rounded p-0.5 hover:bg-red-800/60 transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-red-400"
            aria-label="Dismiss"
            onClick={dismiss}
          >
            <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5">
              <line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" />
            </svg>
          </button>
        </div>
      )}
    </>
  );
};

export default ConnectionStatusBanner;
