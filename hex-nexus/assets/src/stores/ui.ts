/**
 * ui.ts — Global UI state signals (dialogs, modals, toasts).
 */
import { createSignal } from "solid-js";

// Spawn dialog
const [spawnDialogOpen, setSpawnDialogOpen] = createSignal(false);
export { spawnDialogOpen, setSpawnDialogOpen };

// Command palette
const [commandPaletteOpen, setCommandPaletteOpen] = createSignal(false);
export { commandPaletteOpen, setCommandPaletteOpen };
