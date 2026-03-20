/**
 * PaneLeaf.tsx — Renders a pane leaf with tab bar.
 *
 * Single-tab panes show a minimal header. Multi-tab panes show
 * a full tab bar with close buttons and the active tab highlighted.
 */
import { Component, Switch, Match, Show, For, lazy, createMemo } from "solid-js";
import type { PaneLeaf as PaneLeafType, PaneTab } from "../../stores/panes";
import { activePaneId, focusPane, activeTab, switchTab, closeTab } from "../../stores/panes";

// Direct imports for always-loaded views
import ChatView from "../chat/ChatView";
import ProjectOverview from "../project/ProjectOverview";

// Lazy imports for heavier views
const FileTree = lazy(() => import("../project/FileTree"));
const TaskBoard = lazy(() => import("../project/TaskBoard"));
const AgentLog = lazy(() => import("../project/AgentLog"));
const SwarmMonitor = lazy(() => import("../swarm/SwarmMonitor"));
const FleetView = lazy(() => import("../fleet/FleetView"));
const InferencePanel = lazy(() => import("../fleet/InferencePanel"));

/** Render the content for a tab based on its paneType. */
const TabContent: Component<{ tab: PaneTab }> = (props) => (
  <Switch fallback={<PlaceholderPane type={props.tab.paneType} />}>
    <Match when={props.tab.paneType === "project-overview"}>
      <ProjectOverview />
    </Match>
    <Match when={props.tab.paneType === "chat"}>
      <ChatView />
    </Match>
    <Match when={props.tab.paneType === "filetree"}>
      <FileTree projectId={props.tab.props.projectId ?? ""} />
    </Match>
    <Match when={props.tab.paneType === "taskboard"}>
      <TaskBoard swarmId={props.tab.props.swarmId ?? ""} />
    </Match>
    <Match when={props.tab.paneType === "agent-log"}>
      <AgentLog agentId={props.tab.props.agentId ?? ""} />
    </Match>
    <Match when={props.tab.paneType === "swarm-monitor"}>
      <SwarmMonitor swarmId={props.tab.props.swarmId ?? ""} />
    </Match>
    <Match when={props.tab.paneType === "fleet-view"}>
      <FleetView />
    </Match>
    <Match when={props.tab.paneType === "inference"}>
      <InferencePanel />
    </Match>
  </Switch>
);

const PaneLeaf: Component<{ node: PaneLeafType }> = (props) => {
  const isActive = () => activePaneId() === props.node.id;
  const currentTab = createMemo(() => activeTab(props.node));
  const hasTabs = () => props.node.tabs.length > 1;

  return (
    <div
      class="flex flex-col h-full w-full overflow-hidden"
      classList={{
        "ring-1 ring-cyan-500/40": isActive(),
        "ring-1 ring-transparent": !isActive(),
      }}
      onMouseDown={() => focusPane(props.node.id)}
    >
      {/* Tab bar — full when multiple tabs, minimal when single */}
      <Show
        when={hasTabs()}
        fallback={
          <div class="flex h-8 shrink-0 items-center justify-between border-b border-gray-800 bg-gray-900/80 px-3">
            <span class="truncate text-[11px] font-medium text-gray-100">
              {currentTab().title}
            </span>
            <span class="rounded bg-gray-800 px-1.5 py-0.5 text-[9px] uppercase tracking-wider text-gray-300">
              {currentTab().paneType}
            </span>
          </div>
        }
      >
        {/* Multi-tab bar */}
        <div class="flex h-9 shrink-0 items-end border-b border-gray-800 bg-gray-900/80 overflow-x-auto">
          <For each={props.node.tabs}>
            {(tab) => {
              const isTabActive = () => tab.id === props.node.activeTabId;
              return (
                <div
                  class="group flex items-center gap-1.5 border-r border-gray-800 px-3 py-1.5 text-[11px] cursor-pointer transition-colors shrink-0"
                  classList={{
                    "bg-gray-950 text-gray-300 border-b-2 border-b-cyan-500": isTabActive(),
                    "text-gray-300 hover:text-gray-300 hover:bg-gray-800/50": !isTabActive(),
                  }}
                  onClick={() => switchTab(props.node.id, tab.id)}
                >
                  <span class="truncate max-w-[120px]">{tab.title}</span>
                  {/* Close tab button */}
                  <button
                    class="ml-1 rounded p-0.5 opacity-0 group-hover:opacity-100 hover:bg-gray-700 transition-opacity"
                    onClick={(e) => {
                      e.stopPropagation();
                      closeTab(props.node.id, tab.id);
                    }}
                  >
                    <svg class="h-2.5 w-2.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3">
                      <line x1="18" y1="6" x2="6" y2="18" />
                      <line x1="6" y1="6" x2="18" y2="18" />
                    </svg>
                  </button>
                </div>
              );
            }}
          </For>
        </div>
      </Show>

      {/* Pane content — renders active tab */}
      <div class="flex-1 overflow-auto">
        <TabContent tab={currentTab()} />
      </div>
    </div>
  );
};

const PlaceholderPane: Component<{ type: string }> = (props) => (
  <div class="flex h-full items-center justify-center">
    <p class="text-sm text-gray-300">
      {props.type} (coming soon)
    </p>
  </div>
);

export default PaneLeaf;
