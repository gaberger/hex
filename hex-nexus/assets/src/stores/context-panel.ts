import { createSignal } from "solid-js";

export type PanelContent =
  | { type: "default" }
  | { type: "agent-detail"; agentId: string; agentName: string }
  | { type: "swarm-detail"; swarmId: string; swarmName: string }
  | { type: "project-detail"; projectPath: string }
  | { type: "health-detail" }
  | { type: "dep-graph" }
  | { type: "inference" }
  | { type: "fleet" };

const [panelContent, setPanelContent] = createSignal<PanelContent>({ type: "default" });
export { panelContent, setPanelContent };

/** Convenience: reset to default view */
export function resetPanel() {
  setPanelContent({ type: "default" });
}
