// Session store — wired to hex-nexus /api/sessions CRUD API.
// Sessions persist server-side; active session ID cached in localStorage.
// TODO(ADR-044): Migrate to SpacetimeDB chat-relay subscriptions for cross-client sync.

import { createSignal } from "solid-js";
import { addToast } from "./toast";
import { loadChatHistory } from "./chat";

export interface Session {
  id: string;
  name: string;
  createdAt: string;
  messageCount: number;
  status: "active" | "paused" | "completed";
}

// Backend session shape may differ — normalise to our interface
function normalizeSession(raw: any): Session {
  return {
    id: raw.id ?? raw.session_id ?? "",
    name: raw.title ?? raw.name ?? "Untitled",
    createdAt: raw.created_at ?? raw.createdAt ?? new Date().toISOString(),
    messageCount: raw.message_count ?? raw.messageCount ?? 0,
    status: raw.status ?? "active",
  };
}

const [sessions, setSessions] = createSignal<Session[]>([]);
const [activeSessionId, setActiveSessionId] = createSignal<string>("");
const [loading, setLoading] = createSignal(false);

export { sessions, setSessions, activeSessionId, setActiveSessionId, loading };

const STORAGE_KEY = "hex-active-session";

// ── Fetch helpers ──────────────────────────────────────────

async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(path, init);
  if (!res.ok) {
    const body = await res.json().catch(() => ({ error: res.statusText }));
    throw new Error(body.error ?? `HTTP ${res.status}`);
  }
  return res.json();
}

// ── Load sessions from backend ─────────────────────────────

export async function loadSessions(): Promise<void> {
  setLoading(true);
  try {
    const raw = await apiFetch<any[]>("/api/sessions");
    const list = (raw ?? []).map(normalizeSession);
    setSessions(list);

    // Restore active session from localStorage (if it still exists)
    const savedId = localStorage.getItem(STORAGE_KEY);
    if (savedId && list.some((s) => s.id === savedId)) {
      setActiveSessionId(savedId);
    } else if (list.length > 0) {
      setActiveSessionId(list[0].id);
      localStorage.setItem(STORAGE_KEY, list[0].id);
    }
  } catch (e: any) {
    // Backend may be unavailable — fall back to empty list silently
    console.warn("[session] failed to load sessions:", e.message);
    // Seed a local-only session so the UI is usable
    if (sessions().length === 0) {
      const fallback: Session = {
        id: crypto.randomUUID(),
        name: "Local Session",
        createdAt: new Date().toISOString(),
        messageCount: 0,
        status: "active",
      };
      setSessions([fallback]);
      setActiveSessionId(fallback.id);
    }
  } finally {
    setLoading(false);
  }
}

// ── Create session (POST to backend) ───────────────────────

export async function createSession(name?: string): Promise<Session> {
  const title = name || `Session ${sessions().length + 1}`;

  // Optimistically mark previous active session as paused
  setSessions((prev) =>
    prev.map((s) =>
      s.id === activeSessionId() ? { ...s, status: "paused" as const } : s
    )
  );

  try {
    const raw = await apiFetch<any>("/api/sessions", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ projectId: "", title }),
    });
    const session = normalizeSession(raw);
    setSessions((prev) => [session, ...prev]);
    setActiveSessionId(session.id);
    localStorage.setItem(STORAGE_KEY, session.id);
    return session;
  } catch (e: any) {
    // Fall back to local-only session
    addToast("error", `Failed to create session: ${e.message}`);
    const local: Session = {
      id: crypto.randomUUID(),
      name: title,
      createdAt: new Date().toISOString(),
      messageCount: 0,
      status: "active",
    };
    setSessions((prev) => [local, ...prev]);
    setActiveSessionId(local.id);
    localStorage.setItem(STORAGE_KEY, local.id);
    return local;
  }
}

// ── Switch active session ──────────────────────────────────

export async function switchSession(id: string): Promise<void> {
  setActiveSessionId(id);
  localStorage.setItem(STORAGE_KEY, id);

  // Reload chat history for the newly active session
  loadChatHistory(id);

  try {
    // Load the session detail to ensure it exists on the backend
    const raw = await apiFetch<any>(`/api/sessions/${id}`);
    const session = normalizeSession(raw);
    // Update local list with latest server data
    setSessions((prev) =>
      prev.map((s) => (s.id === id ? session : s))
    );
  } catch (e: any) {
    // Non-fatal — session may be local-only
    console.warn("[session] failed to fetch session detail:", e.message);
  }
}

// ── Delete session ─────────────────────────────────────────

export async function deleteSession(id: string): Promise<void> {
  try {
    await apiFetch<any>(`/api/sessions/${id}`, { method: "DELETE" });
  } catch (e: any) {
    addToast("error", `Failed to delete session: ${e.message}`);
  }

  setSessions((prev) => prev.filter((s) => s.id !== id));

  // If we deleted the active session, switch to the first remaining
  if (activeSessionId() === id) {
    const remaining = sessions();
    if (remaining.length > 0) {
      setActiveSessionId(remaining[0].id);
      localStorage.setItem(STORAGE_KEY, remaining[0].id);
    } else {
      setActiveSessionId("");
      localStorage.removeItem(STORAGE_KEY);
    }
  }
}

// ── Initialise on module load ──────────────────────────────

loadSessions();
