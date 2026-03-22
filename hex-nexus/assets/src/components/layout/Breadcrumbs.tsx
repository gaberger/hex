import { Component, For, Show } from "solid-js";
import {
  breadcrumbs,
  navigate,
  type Breadcrumb,
  type Route,
} from "../../stores/router";

const iconPaths: Record<string, string> = {
  hexagon: "M12 2L21 7.5V16.5L12 22L3 16.5V7.5L12 2Z",
  folder:
    "M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z",
  "message-square":
    "M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z",
  "file-text":
    "M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z M14 2v6h6 M16 13H8 M16 17H8 M10 9H8",
  activity: "M22 12h-4l-3 9L9 3l-3 9H2",
  "share-2":
    "M18 8A3 3 0 1 0 18 2 3 3 0 0 0 18 8z M6 15A3 3 0 1 0 6 9 3 3 0 0 0 6 15z M18 22A3 3 0 1 0 18 16 3 3 0 0 0 18 22z",
  bot: "M12 8V4H8 M20 12V8h-4m-4-4v4m0 0h4M8 8h4",
  settings:
    "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z",
  server: "M2 6h20v4H2zm0 8h20v4H2z",
  monitor:
    "M20 3H4a2 2 0 0 0-2 2v10a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2V5a2 2 0 0 0-2-2z M8 21h8 M12 17v4",
};

const BreadcrumbIcon: Component<{ name: string; class?: string }> = (
  props
) => (
  <svg
    class={props.class || "h-3.5 w-3.5"}
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="2"
    stroke-linecap="round"
    stroke-linejoin="round"
  >
    <path d={iconPaths[props.name] || iconPaths.hexagon} />
  </svg>
);

const Breadcrumbs: Component = () => {
  const crumbs = breadcrumbs;
  const isLast = (i: number) => i === crumbs().length - 1;

  return (
    <Show when={crumbs().length > 1}>
    <div class="flex items-center gap-2 border-b border-gray-800 bg-[var(--bg-surface)] px-6 py-2.5">
      <For each={crumbs()}>
        {(crumb, i) => (
          <>
            <Show when={i() > 0}>
              <svg
                class="h-3.5 w-3.5 text-gray-600"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.5"
              >
                <polyline points="9 18 15 12 9 6" />
              </svg>
            </Show>
            <button
              class="flex items-center gap-2 rounded-md px-2.5 py-1 text-sm transition-colors"
              classList={{
                "text-gray-400 hover:text-gray-200 hover:bg-gray-800/50": !isLast(i()),
                "text-gray-100 font-semibold": isLast(i()),
                "cursor-pointer": !!crumb.route && !isLast(i()),
                "cursor-default": isLast(i()),
              }}
              onClick={() => {
                if (crumb.route && !isLast(i())) {
                  navigate(crumb.route);
                }
              }}
            >
              <BreadcrumbIcon
                name={crumb.icon}
                class={`h-3.5 w-3.5 ${isLast(i()) ? "" : "opacity-60"}`}
              />
              {crumb.label}
            </button>
          </>
        )}
      </For>
    </div>
    </Show>
  );
};

export default Breadcrumbs;
