/**
 * toast.ts — Simple toast notification store.
 *
 * Provides addToast / removeToast and a reactive toasts signal.
 */
import { createSignal } from "solid-js";

export interface Toast {
  id: string;
  type: "success" | "error" | "info";
  message: string;
}

const [toasts, setToasts] = createSignal<Toast[]>([]);
export { toasts };

export function addToast(type: Toast["type"], message: string, durationMs = 4000) {
  const id = crypto.randomUUID();
  setToasts((prev) => [...prev, { id, type, message }]);
  setTimeout(() => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, durationMs);
}

export function removeToast(id: string) {
  setToasts((prev) => prev.filter((t) => t.id !== id));
}
