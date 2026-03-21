import { createSignal } from "solid-js";

export type HexMode = "plan" | "build";

const [mode, setMode] = createSignal<HexMode>("plan");
export { mode, setMode };

export function toggleMode() {
  setMode(m => m === "plan" ? "build" : "plan");
}
