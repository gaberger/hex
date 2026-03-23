/**
 * command-history.ts — Tracks command execution history for the CommandOutputPanel.
 *
 * Records every command dispatched from the dashboard (via command palette or
 * action panels), stores results, and provides reactive accessors for display.
 * Subscribes to WebSocket result events for async command completion (ADR-2603231309).
 */
import { createSignal } from "solid-js";

export interface CommandHistoryEntry {
  id: string;
  label: string;
  category: string;
  status: "running" | "success" | "error";
  startedAt: string;
  completedAt?: string;
  result?: string;
  error?: string;
}

const MAX_HISTORY = 50;

const [entries, setEntries] = createSignal<CommandHistoryEntry[]>([]);
const [panelOpen, setPanelOpen] = createSignal(false);

let counter = 0;

/** Record a command execution start. Returns the entry ID for later update. */
export function recordCommandStart(label: string, category: string): string {
  const id = `cmd-${++counter}-${Date.now()}`;
  const entry: CommandHistoryEntry = {
    id,
    label,
    category,
    status: "running",
    startedAt: new Date().toISOString(),
  };
  setEntries((prev) => [entry, ...prev].slice(0, MAX_HISTORY));
  return id;
}

/** Record a successful command result. */
export function recordCommandSuccess(id: string, result?: string) {
  setEntries((prev) =>
    prev.map((e) =>
      e.id === id
        ? { ...e, status: "success" as const, completedAt: new Date().toISOString(), result }
        : e
    )
  );
}

/** Record a command error. */
export function recordCommandError(id: string, error: string) {
  setEntries((prev) =>
    prev.map((e) =>
      e.id === id
        ? { ...e, status: "error" as const, completedAt: new Date().toISOString(), error }
        : e
    )
  );
}

/** Clear all history entries. */
export function clearHistory() {
  setEntries([]);
}

export { entries as commandHistory, panelOpen, setPanelOpen };
