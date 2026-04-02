// hex-sidebar.tsx - OpenCode TUI plugin for hex live status
// Templated with __HEX_NEXUS_URL__ at write time
/* @jsxImportSource solid-js/h */

import type { TuiPluginApi, TuiPluginModule } from "@opencode-ai/plugin/dist/tui";
import { createSignal, onMount, onCleanup } from "solid-js";

const NEXUS_URL = "__HEX_NEXUS_URL__";
const REFRESH_MS = 30000;
const MAX_WIDTH = 38;

function trunc(str: string, len = MAX_WIDTH): string {
  return str && str.length > len ? str.slice(0, len - 1) + "…" : (str || "");
}

interface NexusStatus { name: string; version: string; buildHash: string; }
interface Swarm { id: string; name: string; status: string; tasks: { id: string; status: string }[]; }
interface Adr { id: string; title: string; status: string; }
interface Provider { id: string; model: string; status: string; }

export const tui: TuiPluginModule["tui"] = async (api: TuiPluginApi) => {
  api.slots.register({
    order: 150,
    slots: {
      sidebar_content: () => {
        const [status, setStatus] = createSignal<NexusStatus | null>(null);
        const [swarms, setSwarms] = createSignal<Swarm[]>([]);
        const [adrs, setAdrs] = createSignal<Adr[]>([]);
        const [providers, setProviders] = createSignal<Provider[]>([]);
        const [offline, setOffline] = createSignal(false);

        const refresh = async () => {
          try {
            const [sRes, wRes, aRes, pRes] = await Promise.all([
              fetch(`${NEXUS_URL}/api/status`).catch(() => null),
              fetch(`${NEXUS_URL}/api/hexflo/swarms`).catch(() => null),
              fetch(`${NEXUS_URL}/api/adrs?limit=4`).catch(() => null),
              fetch(`${NEXUS_URL}/api/inference/endpoints`).catch(() => null),
            ]);
            if (!sRes?.ok) { setOffline(true); return; }
            setOffline(false);
            setStatus(await sRes.json());
            if (wRes?.ok) { const d = await wRes.json(); setSwarms(Array.isArray(d) ? d : []); }
            if (aRes?.ok) { const d = await aRes.json(); setAdrs(Array.isArray(d) ? d.slice(0, 4) : []); }
            if (pRes?.ok) { const d = await pRes.json(); setProviders(Array.isArray(d) ? d.slice(0, 3) : []); }
          } catch { setOffline(true); }
        };

        onMount(() => {
          refresh();
          const id = setInterval(refresh, REFRESH_MS);
          onCleanup(() => clearInterval(id));
        });

        const activeSwarms = () => swarms().filter(s => s.status === "active");

        return (
          <div style={{ padding: "4px 8px", "font-size": "12px" }}>
            {offline() ? (
              <div>
                <div style={{ "font-weight": "bold", "margin-bottom": "4px" }}>⬡ hex</div>
                <div style={{ color: "#f87171" }}>nexus offline</div>
              </div>
            ) : (
              <div>
                <div style={{ display: "flex", "align-items": "center", gap: "4px", "margin-bottom": "6px" }}>
                  <span style={{ color: "#22c55e" }}>●</span>
                  <span style={{ "font-weight": "bold" }}>⬡ hex</span>
                  <span style={{ color: "#6b7280", "font-size": "10px" }}>
                    {(status()?.buildHash ?? "").slice(0, 8)}
                  </span>
                </div>

                {activeSwarms().length > 0 && (
                  <div style={{ "margin-bottom": "6px" }}>
                    <div style={{ color: "#a78bfa", "font-weight": "bold", "margin-bottom": "2px" }}>Swarms</div>
                    {activeSwarms().map(s => {
                      const done = s.tasks.filter(t => t.status === "completed").length;
                      return (
                        <div style={{ "font-size": "11px", "padding-left": "4px" }}>
                          {trunc(s.name, 26)} {done}/{s.tasks.length}
                        </div>
                      );
                    })}
                  </div>
                )}

                {adrs().length > 0 && (
                  <div style={{ "margin-bottom": "6px" }}>
                    <div style={{ color: "#fbbf24", "font-weight": "bold", "margin-bottom": "2px" }}>ADRs</div>
                    {adrs().map(a => (
                      <div style={{ "font-size": "11px", "padding-left": "4px", display: "flex", gap: "4px" }}>
                        <span style={{ color: "#6b7280" }}>{trunc(a.id, 14)}</span>
                        <span style={{ color: a.status === "Accepted" ? "#22c55e" : "#9ca3af" }}>
                          {trunc(a.status, 10)}
                        </span>
                      </div>
                    ))}
                  </div>
                )}

                {providers().length > 0 && (
                  <div>
                    <div style={{ color: "#38bdf8", "font-weight": "bold", "margin-bottom": "2px" }}>Providers</div>
                    {providers().map(p => (
                      <div style={{ "font-size": "11px", "padding-left": "4px", display: "flex", gap: "4px" }}>
                        <span style={{ color: p.status === "online" ? "#22c55e" : "#f87171" }}>●</span>
                        <span>{trunc(p.model || p.id, 20)}</span>
                      </div>
                    ))}
                  </div>
                )}
              </div>
            )}
          </div>
        );
      },
    },
  });
};

const server = async () => ({});

export default { tui, server };
