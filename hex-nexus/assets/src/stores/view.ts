import { createSignal } from "solid-js";

export type ViewMode = "chat" | "panes";

const [viewMode, setViewMode] = createSignal<ViewMode>("chat");
export { viewMode, setViewMode };

export function toggleViewMode() {
  setViewMode((m) => (m === "chat" ? "panes" : "chat"));
}
