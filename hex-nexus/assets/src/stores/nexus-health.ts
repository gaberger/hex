/**
 * nexus-health.ts — Polls hex-nexus /api/version for daemon health.
 *
 * Provides reactive signals for: online status, version, uptime.
 * Polls every 10 seconds. Falls back gracefully when nexus is offline.
 */
import { createSignal, onCleanup } from "solid-js";

export interface NexusStatus {
  online: boolean;
  version: string;
  uptime: string;
  pid: number;
  port: number;
  agents: number;
  swarms: number;
  spacetimedb: boolean;
}

const DEFAULT: NexusStatus = {
  online: false,
  version: "—",
  uptime: "—",
  pid: 0,
  port: 5555,
  agents: 0,
  swarms: 0,
  spacetimedb: false,
};

const [nexusStatus, setNexusStatus] = createSignal<NexusStatus>(DEFAULT);
export { nexusStatus };

let _pollTimer: ReturnType<typeof setInterval> | null = null;

async function pollNexus() {
  try {
    const res = await fetch("/api/version", { signal: AbortSignal.timeout(3000) });
    if (!res.ok) {
      setNexusStatus({ ...DEFAULT, online: false });
      return;
    }
    const data = await res.json();
    setNexusStatus({
      online: true,
      version: data.version ?? data.hex_version ?? "—",
      uptime: data.uptime ?? "—",
      pid: data.pid ?? 0,
      port: data.port ?? 5555,
      agents: data.agents ?? data.agent_count ?? 0,
      swarms: data.swarms ?? data.swarm_count ?? 0,
      spacetimedb: data.spacetimedb ?? data.stdb_connected ?? false,
    });
  } catch {
    setNexusStatus({ ...DEFAULT, online: false });
  }
}

/** Start polling nexus health. Call once at app init. */
export function startNexusHealthPoll() {
  if (_pollTimer) return;
  pollNexus(); // immediate first poll
  _pollTimer = setInterval(pollNexus, 10_000);
}

/** Stop polling (cleanup). */
export function stopNexusHealthPoll() {
  if (_pollTimer) {
    clearInterval(_pollTimer);
    _pollTimer = null;
  }
}
