// TODO(ADR-044): Migrate session persistence to SpacetimeDB chat-relay module.
// Currently sessions are in-memory SolidJS signals and lost on page refresh.
// Target: store sessions in SpacetimeDB, subscribe for cross-session visibility.

import { createSignal } from "solid-js";

export interface Session {
  id: string;
  name: string;
  createdAt: string;
  messageCount: number;
  status: "active" | "paused" | "completed";
}

const [sessions, setSessions] = createSignal<Session[]>([
  {
    id: crypto.randomUUID(),
    name: "Current Session",
    createdAt: new Date().toISOString(),
    messageCount: 0,
    status: "active",
  },
]);

const [activeSessionId, setActiveSessionId] = createSignal<string>(sessions()[0]?.id ?? "");

export { sessions, setSessions, activeSessionId, setActiveSessionId };

export function createSession(name?: string): Session {
  const session: Session = {
    id: crypto.randomUUID(),
    name: name || `Session ${sessions().length + 1}`,
    createdAt: new Date().toISOString(),
    messageCount: 0,
    status: "active",
  };
  // Mark previous active as paused
  setSessions(prev => prev.map(s =>
    s.id === activeSessionId() ? { ...s, status: "paused" as const } : s
  ));
  setSessions(prev => [session, ...prev]);
  setActiveSessionId(session.id);
  return session;
}
