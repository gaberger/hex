/**
 * ProjectLayout.tsx — Shared layout wrapper for all project sub-pages.
 *
 * Provides: horizontal tab nav bar + content area.
 * The permanent nav bar in App.tsx handles project navigation,
 * so ProjectSidebar is not rendered here.
 */
import { type Component, type JSX, createMemo } from "solid-js";
import { route, navigate } from "../../stores/router";

interface NavTab {
  label: string;
  page: string;
  icon: string; // SVG path d attribute
}

const tabs: NavTab[] = [
  {
    label: "Overview",
    page: "project",
    icon: "M3 9l9-7 9 7v11a2 2 0 01-2 2H5a2 2 0 01-2-2z M9 22V12h6v10",
  },
  {
    label: "Files",
    page: "project-files",
    icon: "M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z",
  },
  {
    label: "ADRs",
    page: "project-adrs",
    icon: "M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z M14 2v6h6 M16 13H8 M16 17H8 M10 9H8",
  },
  {
    label: "Health",
    page: "project-health",
    icon: "M22 12h-4l-3 9L9 3l-3 9H2",
  },
  {
    label: "Graph",
    page: "project-graph",
    icon: "M18 20V10 M12 20V4 M6 20v-6",
  },
  {
    label: "Chat",
    page: "project-chat",
    icon: "M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2z",
  },
  {
    label: "Config",
    page: "project-config",
    icon: "M12.22 2h-.44a2 2 0 00-2 2v.18a2 2 0 01-1 1.73l-.43.25a2 2 0 01-2 0l-.15-.08a2 2 0 00-2.73.73l-.22.38a2 2 0 00.73 2.73l.15.1a2 2 0 011 1.72v.51a2 2 0 01-1 1.74l-.15.09a2 2 0 00-.73 2.73l.22.38a2 2 0 002.73.73l.15-.08a2 2 0 012 0l.43.25a2 2 0 011 1.73V20a2 2 0 002 2h.44a2 2 0 002-2v-.18a2 2 0 011-1.73l.43-.25a2 2 0 012 0l.15.08a2 2 0 002.73-.73l.22-.39a2 2 0 00-.73-2.73l-.15-.08a2 2 0 01-1-1.74v-.5a2 2 0 011-1.74l.15-.09a2 2 0 00.73-2.73l-.22-.38a2 2 0 00-2.73-.73l-.15.08a2 2 0 01-2 0l-.43-.25a2 2 0 01-1-1.73V4a2 2 0 00-2-2z M12 15a3 3 0 100-6 3 3 0 000 6z",
  },
];

const ProjectLayout: Component<{ children: JSX.Element }> = (props) => {
  const currentPage = createMemo(() => route().page);
  const projectId = createMemo(() => (route() as any).projectId ?? "");

  const isActive = (tab: NavTab) => {
    const page = currentPage();
    if (tab.page === "project") return page === "project";
    if (tab.page === "project-adrs") return page === "project-adrs" || page === "project-adr-detail";
    if (tab.page === "project-config") return page === "project-config";
    if (tab.page === "project-files") return page === "project-files" || page === "project-file";
    return page === tab.page;
  };

  const handleNav = (tab: NavTab) => {
    const pid = projectId();
    switch (tab.page) {
      case "project":
        navigate({ page: "project", projectId: pid });
        break;
      case "project-adrs":
        navigate({ page: "project-adrs", projectId: pid });
        break;
      case "project-chat":
        navigate({ page: "project-chat", projectId: pid });
        break;
      case "project-health":
        navigate({ page: "project-health", projectId: pid });
        break;
      case "project-graph":
        navigate({ page: "project-graph", projectId: pid });
        break;
      case "project-files":
        navigate({ page: "project-files", projectId: pid });
        break;
      case "project-config":
        navigate({ page: "project-config", projectId: pid, section: "blueprint" });
        break;
    }
  };

  return (
    <div class="flex flex-1 flex-col overflow-hidden">
      {/* Project nav tabs */}
      <div class="flex items-center gap-0 border-b border-gray-800 bg-gray-950 px-4 shrink-0">
        {tabs.map((tab) => (
          <button
            class="flex items-center gap-1.5 px-3 py-2.5 text-[11px] font-medium transition-colors tracking-wide"
            classList={{
              "text-cyan-400 border-b-2 border-cyan-500": isActive(tab),
              "text-gray-500 border-b-2 border-transparent hover:text-gray-300": !isActive(tab),
            }}
            onClick={() => handleNav(tab)}
          >
            <svg
              class="h-3.5 w-3.5"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path d={tab.icon} />
            </svg>
            {tab.label}
          </button>
        ))}
      </div>

      {/* Route content */}
      <div class="flex flex-1 overflow-hidden">
        {props.children}
      </div>
    </div>
  );
};

export default ProjectLayout;
