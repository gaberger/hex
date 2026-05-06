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

// Polyfill for crypto.randomUUID (not available in all browser contexts)
function generateUUID(): string {
  if (typeof crypto !== 'undefined' && crypto.randomUUID) {
    return crypto.randomUUID();
  }
  // Fallback: simple UUID v4 implementation
  return 'xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx'.replace(/[xy]/g, (c) => {
    const r = (Math.random() * 16) | 0;
    const v = c === 'x' ? r : (r & 0x3) | 0x8;
    return v.toString(16);
  });
}

export function addToast(type: Toast["type"], message: string, durationMs = 4000) {
  const id = generateUUID();
  setToasts((prev) => [...prev, { id, type, message }]);
  setTimeout(() => {
    setToasts((prev) => prev.filter((t) => t.id !== id));
  }, durationMs);
}

export function removeToast(id: string) {
  setToasts((prev) => prev.filter((t) => t.id !== id));
}
