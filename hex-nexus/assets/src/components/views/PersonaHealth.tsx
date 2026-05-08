/**
 * PersonaHealth.tsx — supervisor health for executive personas.
 *
 * Reads /api/merge/personas (persona_pool joined to persona_health). Shows
 * pause state, recent_failures, last_failure_status, and a live banned_until
 * countdown. Mirrors the OTP-style supervisor feedback loop documented in
 * feedback_supervisor_in_stdb.md.
 */

import { Component, For, Show, createSignal, onMount, onCleanup } from "solid-js";
import { restClient } from "../../services/rest-client";

interface PersonaHealthRow {
  recent_failures: number;
  last_failure_at: string;
  last_failure_model: string;
  last_failure_status: number;
  banned_until: string;
}

interface PersonaRow {
  role: string;
  display_name: string;
  tier: string;
  paused: boolean;
  last_tick_at: string;
  health: PersonaHealthRow | null;
}

interface PersonaList {
  personas: PersonaRow[];
}

const REFRESH_MS = 4000;

// Parses STDB Timestamp { __timestamp_micros_since_unix_epoch__: N } or
// returns null if N is 0/empty/invalid.
const parseMicros = (ts: string | null | undefined): number | null => {
  if (!ts) return null;
  const m = ts.match(/__timestamp_micros_since_unix_epoch__:\s*(-?\d+)/);
  if (!m) return null;
  const n = Number(m[1]);
  if (!Number.isFinite(n) || n === 0) return null;
  return n;
};

const PersonaHealth: Component = () => {
  const [personas, setPersonas] = createSignal<PersonaRow[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [now, setNow] = createSignal(Date.now() * 1000);

  let timer: ReturnType<typeof setInterval> | null = null;
  let clock: ReturnType<typeof setInterval> | null = null;

  const refresh = async () => {
    try {
      const data: PersonaList = await restClient.get("/api/merge/personas");
      setPersonas(data.personas || []);
      setError(null);
    } catch (e: any) {
      setError(e?.message || String(e));
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    refresh();
    timer = setInterval(refresh, REFRESH_MS);
    clock = setInterval(() => setNow(Date.now() * 1000), 1000);
  });
  onCleanup(() => {
    if (timer) clearInterval(timer);
    if (clock) clearInterval(clock);
  });

  const remaining = (untilMicros: number | null): string | null => {
    if (!untilMicros) return null;
    const deltaMs = (untilMicros - now()) / 1000;
    if (deltaMs <= 0) return null;
    const s = Math.ceil(deltaMs / 1000);
    if (s < 60) return `${s}s`;
    if (s < 3600) return `${Math.floor(s / 60)}m ${s % 60}s`;
    return `${Math.floor(s / 3600)}h ${Math.floor((s % 3600) / 60)}m`;
  };

  const tickAge = (ts: string): string => {
    const m = parseMicros(ts);
    if (!m) return "—";
    const ageMs = (now() - m) / 1000;
    if (ageMs < 0) return "—";
    const s = Math.floor(ageMs / 1000);
    if (s < 60) return `${s}s ago`;
    if (s < 3600) return `${Math.floor(s / 60)}m ago`;
    return `${Math.floor(s / 3600)}h ago`;
  };

  const stateBadge = (p: PersonaRow): { label: string; cls: string } => {
    const banUntil = parseMicros(p.health?.banned_until || "");
    const r = remaining(banUntil);
    if (r) return { label: `banned ${r}`, cls: "bg-red-900 text-red-300" };
    if (p.paused) return { label: "paused", cls: "bg-yellow-900 text-yellow-300" };
    if ((p.health?.recent_failures || 0) >= 3)
      return { label: "warning", cls: "bg-orange-900 text-orange-300" };
    return { label: "ready", cls: "bg-green-900 text-green-300" };
  };

  return (
    <div class="flex flex-col bg-gray-950 min-h-screen text-gray-100">
      <div class="p-6 border-b border-gray-800">
        <h1 class="text-2xl font-bold mb-1">Persona Health</h1>
        <p class="text-gray-400 text-sm">
          OTP-style supervisor · STDB-resident · auto-ban / auto-recover
        </p>
      </div>

      <Show when={error()}>
        <div class="p-4 bg-red-950/40 border-b border-red-900 text-red-300 text-sm">
          {error()}
        </div>
      </Show>

      <Show when={loading()}>
        <div class="p-6 text-gray-500">Loading persona pool…</div>
      </Show>

      <Show when={!loading() && personas().length === 0}>
        <div class="p-6 text-gray-500">
          No personas registered. Did <code>persona_init</code> run on startup?
        </div>
      </Show>

      <div class="flex-1 overflow-y-auto px-6 py-4">
        <table class="w-full text-sm">
          <thead>
            <tr class="text-left text-gray-500 uppercase text-xs border-b border-gray-800">
              <th class="py-2 pr-4">role</th>
              <th class="py-2 pr-4">tier</th>
              <th class="py-2 pr-4">state</th>
              <th class="py-2 pr-4">recent failures</th>
              <th class="py-2 pr-4">last status</th>
              <th class="py-2 pr-4">last model</th>
              <th class="py-2 pr-4">last tick</th>
            </tr>
          </thead>
          <tbody>
            <For each={personas()}>
              {(p) => {
                const b = stateBadge(p);
                return (
                  <tr class="border-b border-gray-900/50 hover:bg-gray-900/30">
                    <td class="py-2 pr-4">
                      <div class="text-cyan-400 font-mono">{p.role}</div>
                      <div class="text-gray-500 text-xs">{p.display_name}</div>
                    </td>
                    <td class="py-2 pr-4 text-gray-400">{p.tier}</td>
                    <td class="py-2 pr-4">
                      <span class={`px-2 py-0.5 rounded text-xs ${b.cls}`}>
                        {b.label}
                      </span>
                    </td>
                    <td class="py-2 pr-4 text-gray-300">
                      {p.health?.recent_failures ?? 0}
                    </td>
                    <td class="py-2 pr-4 text-gray-400">
                      {p.health?.last_failure_status || "—"}
                    </td>
                    <td class="py-2 pr-4 text-gray-500 font-mono text-xs truncate max-w-xs">
                      {p.health?.last_failure_model || "—"}
                    </td>
                    <td class="py-2 pr-4 text-gray-400 text-xs">
                      {tickAge(p.last_tick_at)}
                    </td>
                  </tr>
                );
              }}
            </For>
          </tbody>
        </table>
      </div>
    </div>
  );
};

export default PersonaHealth;
