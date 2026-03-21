/**
 * ToastContainer.tsx — Fixed overlay showing toast notifications.
 *
 * Renders in the bottom-right corner. Toasts auto-dismiss after 4s
 * and can be manually closed.
 */
import { Component, For } from "solid-js";
import { toasts, removeToast, type Toast } from "../../stores/toast";

function iconForType(type: Toast["type"]) {
  if (type === "success") {
    return (
      <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <polyline points="20 6 9 17 4 12" />
      </svg>
    );
  }
  if (type === "error") {
    return (
      <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
        <circle cx="12" cy="12" r="10" />
        <line x1="15" y1="9" x2="9" y2="15" />
        <line x1="9" y1="9" x2="15" y2="15" />
      </svg>
    );
  }
  // info
  return (
    <svg class="h-4 w-4 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
      <circle cx="12" cy="12" r="10" />
      <line x1="12" y1="16" x2="12" y2="12" />
      <line x1="12" y1="8" x2="12.01" y2="8" />
    </svg>
  );
}

function styleForType(type: Toast["type"]): string {
  if (type === "success") return "bg-green-900/90 border-green-700 text-green-200";
  if (type === "error") return "bg-red-900/90 border-red-700 text-red-200";
  return "bg-gray-800/90 border-gray-700 text-gray-200";
}

const ToastContainer: Component = () => {
  return (
    <div class="fixed bottom-4 right-4 z-50 flex flex-col gap-2">
      <For each={toasts()}>
        {(toast) => (
          <div
            class={`rounded-lg border px-4 py-3 text-sm shadow-lg flex items-center gap-3 min-w-[280px] max-w-[400px] animate-toast-in ${styleForType(toast.type)}`}
          >
            {iconForType(toast.type)}
            <span class="flex-1">{toast.message}</span>
            <button
              class="shrink-0 rounded p-0.5 opacity-60 hover:opacity-100 transition-opacity"
              onClick={() => removeToast(toast.id)}
            >
              <svg class="h-3.5 w-3.5" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <line x1="18" y1="6" x2="6" y2="18" />
                <line x1="6" y1="6" x2="18" y2="18" />
              </svg>
            </button>
          </div>
        )}
      </For>
    </div>
  );
};

export default ToastContainer;
