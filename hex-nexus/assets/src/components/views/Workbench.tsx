/**
 * Workbench.tsx — the home surface for the single-agent workflow (ADR-2606061359).
 *
 * The unit of work is a direct run: task → one agent → evidence → commit.
 * This combines (1) a New Run composer that launches `hex do` from the UI
 * (POST /api/direct/execute), (2) a live graph-context panel for the targeted
 * file (POST /api/graph/context — trace consumers before you launch), and
 * (3) the runs feed (GET /api/direct/runs). Replaces the org-sim Mission Control.
 */
import { Component, For, Show, createSignal, onMount, onCleanup } from "solid-js";
import { restClient } from "../../services/rest-client";

interface DirectRun {
  id: number;
  agent: string;
  started_at: string;
  instruction: string;
  file: string;
  model: string;
  ok: boolean;
  attempts: number;
  evidence_passed: boolean;
  committed: string | null;
  duration_ms: number;
  error: string | null;
}
interface RunsResponse {
  summary: { total: number; passed: number; failed: number; committed: number; pass_rate: number };
  runs: DirectRun[];
}

const Stat: Component<{ label: string; value: any; tone?: string }> = (p) => (
  <div class="px-4 py-3 rounded bg-gray-900/50 border border-gray-800 min-w-[96px]">
    <div
      class="text-2xl font-semibold"
      classList={{
        "text-green-400": p.tone === "green",
        "text-red-400": p.tone === "red",
        "text-cyan-400": p.tone === "cyan",
      }}
    >
      {p.value}
    </div>
    <div class="text-xs text-gray-500 mt-1">{p.label}</div>
  </div>
);

const Workbench: Component = () => {
  // ── composer ──
  const [instruction, setInstruction] = createSignal("");
  const [file, setFile] = createSignal("");
  const [evidence, setEvidence] = createSignal("");
  const [model, setModel] = createSignal("");
  const [running, setRunning] = createSignal(false);
  const [runMsg, setRunMsg] = createSignal<{ ok: boolean; text: string } | null>(null);

  // ── graph context for the targeted file ──
  const [ctx, setCtx] = createSignal<string>("");
  const loadCtx = async () => {
    const f = file().trim();
    if (!f) { setCtx(""); return; }
    try {
      const r = await restClient.post<{ markdown?: string }>("/api/graph/context", { path: ".", target: f });
      setCtx(r.markdown ?? "");
    } catch {
      setCtx("");
    }
  };

  const launch = async (e: Event) => {
    e.preventDefault();
    if (!instruction().trim() || !file().trim() || !evidence().trim()) {
      setRunMsg({ ok: false, text: "instruction, file, and evidence are all required" });
      return;
    }
    setRunning(true);
    setRunMsg(null);
    try {
      const body: any = { instruction: instruction(), file: file(), evidence: evidence() };
      if (model().trim()) body.model = model().trim();
      const r = await restClient.post<any>("/api/direct/execute", body);
      const ok = !!r.ok;
      const committed = r.committed ? ` · commit ${String(r.committed).slice(0, 8)}` : "";
      setRunMsg({
        ok,
        text: ok
          ? `✓ evidence passed${committed} (${r.attempts} attempt${r.attempts === 1 ? "" : "s"})`
          : `✗ ${r.error ?? "evidence did not pass"} (${r.attempts} attempt${r.attempts === 1 ? "" : "s"})`,
      });
      void load();
    } catch (e: any) {
      setRunMsg({ ok: false, text: String(e?.message ?? e) });
    } finally {
      setRunning(false);
    }
  };

  // ── runs feed ──
  const [data, setData] = createSignal<RunsResponse | null>(null);
  const [err, setErr] = createSignal<string | null>(null);
  let timer: any;
  const load = async () => {
    try {
      setData(await restClient.get<RunsResponse>("/api/direct/runs"));
      setErr(null);
    } catch (e: any) {
      setErr(String(e?.message ?? e));
    }
  };
  onMount(() => { void load(); timer = setInterval(load, 4000); });
  onCleanup(() => clearInterval(timer));

  return (
    <div class="flex flex-1 flex-col overflow-auto p-4 gap-4 text-sm text-gray-200">
      <div>
        <h1 class="text-lg font-semibold text-gray-100">Workbench</h1>
        <p class="text-gray-500 text-xs">task → one agent → evidence → commit. Launch a run, watch it land.</p>
      </div>

      <div class="grid grid-cols-1 lg:grid-cols-2 gap-4">
        {/* New Run composer */}
        <form class="rounded border border-gray-800 bg-gray-900/50 p-3 flex flex-col gap-2" onSubmit={launch}>
          <div class="text-xs uppercase tracking-wide text-gray-500">New run</div>
          <textarea
            class="rounded border border-gray-700 bg-gray-900 px-2 py-1 text-xs h-16 resize-y"
            placeholder="Instruction — what to do, in plain language"
            value={instruction()}
            onInput={(e) => setInstruction(e.currentTarget.value)}
          />
          <input
            class="rounded border border-gray-700 bg-gray-900 px-2 py-1 text-xs"
            placeholder="File (repo-relative, e.g. hex-graph/src/model.rs)"
            value={file()}
            onInput={(e) => setFile(e.currentTarget.value)}
            onBlur={loadCtx}
          />
          <input
            class="rounded border border-gray-700 bg-gray-900 px-2 py-1 text-xs"
            placeholder='Evidence command (must exit 0, e.g. "cargo check -p hex-graph")'
            value={evidence()}
            onInput={(e) => setEvidence(e.currentTarget.value)}
          />
          <input
            class="rounded border border-gray-700 bg-gray-900 px-2 py-1 text-xs"
            placeholder="Model (optional override)"
            value={model()}
            onInput={(e) => setModel(e.currentTarget.value)}
          />
          <div class="flex items-center gap-3">
            <button
              type="submit"
              class="rounded bg-cyan-600 hover:bg-cyan-500 px-3 py-1 text-xs font-medium disabled:opacity-50"
              disabled={running()}
            >
              {running() ? "Running…" : "Run"}
            </button>
            <Show when={runMsg()}>
              {(m) => (
                <span class="text-xs" classList={{ "text-green-400": m().ok, "text-red-400": !m().ok }}>
                  {m().text}
                </span>
              )}
            </Show>
          </div>
        </form>

        {/* Graph context for the targeted file */}
        <div class="rounded border border-gray-800 bg-gray-900/50 p-3 flex flex-col gap-2">
          <div class="text-xs uppercase tracking-wide text-gray-500">File context (consumers · community)</div>
          <Show when={ctx()} fallback={<span class="text-gray-600 text-xs">Enter a file above to see its graph neighbourhood before launching.</span>}>
            <pre class="text-[11px] text-gray-300 whitespace-pre-wrap overflow-auto max-h-72 leading-snug">{ctx()}</pre>
          </Show>
        </div>
      </div>

      {/* Runs feed */}
      <Show when={data()}>
        {(d) => (
          <div class="flex flex-col gap-3">
            <div class="flex gap-3 flex-wrap">
              <Stat label="Runs" value={d().summary.total} />
              <Stat label="Passed" value={d().summary.passed} tone="green" />
              <Stat label="Failed" value={d().summary.failed} tone="red" />
              <Stat label="Committed" value={d().summary.committed} tone="cyan" />
              <Stat label="Pass rate" value={`${Math.round(d().summary.pass_rate * 100)}%`} />
            </div>
            <div class="rounded border border-gray-800 overflow-auto">
              <table class="w-full text-xs">
                <thead class="text-gray-500 border-b border-gray-800">
                  <tr>
                    <th class="text-left px-2 py-1">Agent</th>
                    <th class="text-left px-2 py-1">Task</th>
                    <th class="text-left px-2 py-1">File</th>
                    <th class="text-left px-2 py-1">Evidence</th>
                    <th class="text-left px-2 py-1">Commit</th>
                    <th class="text-right px-2 py-1">Try</th>
                  </tr>
                </thead>
                <tbody>
                  <For each={d().runs}>
                    {(r) => (
                      <tr class="border-b border-gray-900 hover:bg-gray-900/40">
                        <td class="px-2 py-1 text-gray-400">{r.agent}</td>
                        <td class="px-2 py-1 truncate max-w-[28ch]">{r.instruction}</td>
                        <td class="px-2 py-1 text-gray-500 truncate max-w-[24ch]">{r.file}</td>
                        <td class="px-2 py-1">
                          <span classList={{ "text-green-400": r.evidence_passed, "text-red-400": !r.evidence_passed }}>
                            {r.evidence_passed ? "✓ pass" : "✗ fail"}
                          </span>
                        </td>
                        <td class="px-2 py-1 text-cyan-400">{r.committed ? r.committed.slice(0, 8) : "—"}</td>
                        <td class="px-2 py-1 text-right text-gray-500">{r.attempts}</td>
                      </tr>
                    )}
                  </For>
                </tbody>
              </table>
            </div>
          </div>
        )}
      </Show>
      <Show when={err()}><div class="text-red-400 text-xs">{err()}</div></Show>
    </div>
  );
};

export default Workbench;
