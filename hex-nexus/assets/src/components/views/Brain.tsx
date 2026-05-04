/**
 * Brain.tsx — wp-brain-dashboard, full three-pane layout.
 *
 * Page layout:
 *   ┌─────────────────────────────────────────────────────────┐
 *   │  TEAM rail │  CENTER (Kanban + Decisions + Swarms +    │
 *   │  (left)    │           Health)         │  CHAT (right) │
 *   └─────────────────────────────────────────────────────────┘
 *   │  EVENT FEED (collapsible bottom strip)                 │
 *   └─────────────────────────────────────────────────────────┘
 *
 * Status of each pane:
 *   - TeamRail        : live (groups 25 personas by category, online dots from /api/hex-agents)
 *   - KanbanLanes     : live (projects /api/swarms/active tasks into 4 lanes)
 *   - DecisionsPanel  : live (reuses /api/decisions from M1)
 *   - SwarmsPanel     : live (/api/swarms/active)
 *   - HealthPanel     : live (/api/health + /api/sched/improver/status)
 *   - ChatPanel       : input shell only — full WebSocket dispatch lands in M3
 *   - EventFeed       : static placeholder — STDB subscription wiring lands later
 */
import { Component, For, Show, createEffect, createMemo, createSignal, onCleanup, onMount } from "solid-js";
import { restClient } from "../../services/rest-client";
import MarkdownContent from "../chat/MarkdownContent";

// ── Persona registry (mirrors hex-cli/assets/agents/hex/hex/) ────────────────
// Categories match the org-chart in the operator briefing. Order intentional.

interface Persona {
  name: string;
  category: "PRODUCT" | "ENGINEERING" | "QUALITY" | "DESIGN" | "OPS";
  color: string; // tailwind text-color class
  /** One-line summary, shown on hover in the team rail. */
  tagline: string;
  /** Two-to-three sentence description — shown when the persona is selected. */
  description: string;
  /** When the operator should invoke this agent vs. its siblings. */
  whenToUse: string;
}

// Roster mirrors hex-cli/assets/agents/hex/hex/<name>.yml. Descriptions
// distilled from each YAML's `description:` field plus the operator briefing
// so the user can see the pipeline ordering at a glance:
//   PRODUCT:   dependency-analyst → pm-agent → behavioral-spec-writer → planner → feature-developer
//   ENG:       feature-developer → swarm-coordinator → (hex-coder | hex-tester | hex-fixer | hex-documenter | hex-ux | rust-refactorer) → integrator
//   QUALITY:   hex-reviewer (in-loop) → adversarial-red + adversarial-blue → validation-judge → adr-reviewer + dead-code-analyzer + scaffold-validator
//   DESIGN:    cli-designer + ux-designer (read-only critique)
//   OPS:       dev-tracker (resume) + status-monitor (live)
const PERSONAS: Persona[] = [
  // ── PRODUCT — what to build, why, in what order ────────────────────────
  { name: "dependency-analyst",    category: "PRODUCT",     color: "text-cyan-400",
    tagline: "Tech-stack picker — runs BEFORE planner",
    description: "Analyzes problem requirements to recommend optimal language/library combinations and cross-language communication patterns per adapter boundary.",
    whenToUse: "When you're starting fresh and don't know which language/library/runtime fits each component. Output feeds the planner." },
  { name: "pm-agent",              category: "PRODUCT",     color: "text-purple-400",
    tagline: "ADR-vs-workplan classifier",
    description: "Decides whether a request introduces an architectural commitment (new port, adapter, external dependency, persistence backend, trust tier) → ADR-first, or is tactical → workplan-only.",
    whenToUse: "Default entry point for any new request. Don't pre-decide ADR-or-workplan; let pm-agent classify and route." },
  { name: "behavioral-spec-writer",category: "PRODUCT",     color: "text-green-400",
    tagline: "User-facing specs BEFORE code",
    description: "Writes BehavioralSpec[] from a problem statement using domain knowledge. Specs describe user-facing behavior only — no function names, no internal state.",
    whenToUse: "Before any feature codegen. The specs become validation-judge's independent oracle that catches 'tests mirror the bug' failures." },
  { name: "planner",               category: "PRODUCT",     color: "text-blue-400",
    tagline: "Decomposes into adapter-bounded task graph",
    description: "Breaks high-level requirements into a dependency-ordered workplan. One task = one adapter boundary = one git worktree. Max 8 parallel.",
    whenToUse: "After pm-agent classifies as workplan-only/both. Writes docs/workplans/wp-<slug>.json the swarm-coordinator can dispatch." },
  { name: "feature-developer",     category: "PRODUCT",     color: "text-purple-400",
    tagline: "Top-level 7-phase lifecycle orchestrator",
    description: "Drives SPECS → PLAN → WORKTREES → CODE → VALIDATE → INTEGRATE → FINALIZE end-to-end. Coordinates spec-writer, planner, swarm, judge, integrator.",
    whenToUse: "When you have a feature concept and want the full pipeline run for you. The 'do the whole thing' button." },

  // ── ENGINEERING — does the work ────────────────────────────────────────
  { name: "swarm-coordinator",     category: "ENGINEERING", color: "text-cyan-400",
    tagline: "Parallel-agent dispatcher",
    description: "Initializes HexFlo swarm, assigns workplan tasks to hex-coder agents in parallel worktrees, monitors heartbeats, reassigns on failure, triggers integration.",
    whenToUse: "When the planner's workplan is ready and you want it executed in parallel. Usually invoked by feature-developer." },
  { name: "hex-coder",             category: "ENGINEERING", color: "text-green-400",
    tagline: "Polyglot TDD code-gen in ONE adapter",
    description: "Generates production code within a single hexagonal adapter boundary. TS/Go/Rust. TDD red-green-refactor + per-language compile/lint/test feedback loop. Worktree-isolated.",
    whenToUse: "The default IC for adapter implementation. Never crosses adapter boundaries — one task per invocation." },
  { name: "hex-tester",            category: "ENGINEERING", color: "text-green-400",
    tagline: "London-school unit tests for ONE file",
    description: "Generates unit tests via local Ollama. Mocks via deps pattern (NEVER mock.module() per ADR-014). Covers happy + error + edge cases for every public method.",
    whenToUse: "When hex-coder needs test scaffolding, or when you want to add tests to existing code without rewriting." },
  { name: "hex-fixer",             category: "ENGINEERING", color: "text-orange-400",
    tagline: "Surgical compile/lint/test error fixes",
    description: "Targeted fixes only — no refactoring, no surrounding cleanup. Escalates to Sonnet after 3 failed attempts.",
    whenToUse: "When hex-coder's feedback loop hits max iterations and escalates, or when CI is red and you want it green fast." },
  { name: "hex-documenter",        category: "ENGINEERING", color: "text-yellow-400",
    tagline: "Doc-comment generator (read-only on API)",
    description: "Adds JSDoc/TSDoc/rustdoc/godoc to one file or module. Never alters function signatures or public API.",
    whenToUse: "When public symbols lack docs. Bounded to doc comments — won't add code." },
  { name: "hex-ux",                category: "ENGINEERING", color: "text-pink-400",
    tagline: "Primary-adapter UI micro-fixes",
    description: "Applies a11y / contrast / loading-empty-error states inside ONE primary adapter. Bounded to src/adapters/primary/ — never crosses boundaries.",
    whenToUse: "After @ux-designer produces a UXDesignReport — hex-ux applies it. Distinct from ux-designer (read-only critique)." },
  { name: "rust-refactorer",       category: "ENGINEERING", color: "text-orange-400",
    tagline: "Autonomous Rust refactoring (worktree)",
    description: "Surgical Rust refactors with worktree isolation. Preserves public API. Runs cargo check + clippy + test after every change.",
    whenToUse: "Rust-specific cleanup. Module splits, extract-function refactors. Not for cross-language work." },
  { name: "integrator",            category: "ENGINEERING", color: "text-yellow-400",
    tagline: "Worktree merge captain",
    description: "Merges feature worktrees back to main in dependency order (domain → ports → secondary → primary → usecases → integration). Uses hex worktree merge, never raw git checkout. Resolves conflicts; runs full suite.",
    whenToUse: "After all hex-coder agents finish their adapter tasks. Closes the feature lifecycle." },

  // ── QUALITY — says ship-or-don't ───────────────────────────────────────
  { name: "hex-reviewer",          category: "QUALITY",     color: "text-cyan-400",
    tagline: "Fast in-loop quality gut-check",
    description: "Local Ollama. Boundary check + pattern review + anti-pattern flags. Not a pre-merge gate — for in-development pair-review feedback.",
    whenToUse: "Quick 'look this over before I commit'. Cheaper and faster than the adversarial duo." },
  { name: "adversarial-red",       category: "QUALITY",     color: "text-red-400",
    tagline: "Security/hex-boundary skeptic (Anthropic)",
    description: "provider_lock: anthropic. Hunts hex-rule violations, leaked secrets, autonomy escapes, supply-chain drift, config trust issues. Refuses to run if blue is on the same provider — same provider = shared blindspots.",
    whenToUse: "Pre-merge audit, post-feature work, post-migration. Always paired with @adversarial-blue." },
  { name: "adversarial-blue",      category: "QUALITY",     color: "text-blue-400",
    tagline: "Correctness/UX skeptic (OpenAI/local)",
    description: "provider_lock: openai_or_local. Hunts test-mirror-bug, error-message lies, sign-convention reversals, spec drift. Distinct training biases from red — catches what red misses.",
    whenToUse: "Always paired with @adversarial-red. The two reports go to validation-judge for arbitration." },
  { name: "validation-judge",      category: "QUALITY",     color: "text-red-400",
    tagline: "Final PASS/FAIL verdict + arbitrates red+blue",
    description: "Behavioral specs + property tests (fast-check, not example-based) + smoke + sign-convention + boundary check. PASS at score ≥80, else FAIL with specific fixes. Phase 6a arbitrates red + blue (verifies provider divergence).",
    whenToUse: "Last gate before merge. After both adversaries report. Blocks deployment on FAIL." },
  { name: "adr-reviewer",          category: "QUALITY",     color: "text-yellow-400",
    tagline: "ADR structural + drift validator",
    description: "Validates ADR completeness (Status/Context/Decision/Consequences/Alternatives), legitimate status transitions, cross-references. Flags code that contradicts accepted ADRs.",
    whenToUse: "After writing an ADR or before merging code that touches architectural surfaces." },
  { name: "dead-code-analyzer",    category: "QUALITY",     color: "text-orange-400",
    tagline: "Orphan + unused-export hunter",
    description: "Workspace-wide tree-sitter L1 dependency graph. Orphaned adapter → CRITICAL, unused public export → MEDIUM, cross-adapter import → CRITICAL.",
    whenToUse: "After deletions/refactors to verify nothing dangling. Periodic hygiene." },
  { name: "scaffold-validator",    category: "QUALITY",     color: "text-yellow-400",
    tagline: "'Is this app actually runnable?'",
    description: "Checks README + start script + .env.example AND actually runs the dev command. PASS only when the app starts.",
    whenToUse: "After /hex-scaffold or any new project generation. Closes the 'compiles but doesn't work' gap." },

  // ── DESIGN — read-only critique ────────────────────────────────────────
  { name: "cli-designer",          category: "DESIGN",      color: "text-cyan-400",
    tagline: "CLI surface design reviewer (no code-write)",
    description: "Critiques hex --help ergonomics, flag conventions (kebab-case, --json, --dry-run), alias hierarchies, error-message shape. Emits CLIDesignReport.",
    whenToUse: "Before adding a new CLI command, or to audit existing surfaces. Pair with @hex-coder to apply the report." },
  { name: "ux-designer",           category: "DESIGN",      color: "text-pink-400",
    tagline: "Solid+Tailwind dashboard reviewer (no code-write)",
    description: "Visual hierarchy + WCAG 2.1 AA + state-flow (loading/empty/error all three) + real-time pacing ≥500ms. Cites Nielsen heuristics. Emits UXDesignReport.",
    whenToUse: "Before adding a new dashboard surface, or for a11y/contrast audits. Pair with @hex-ux to apply." },

  // ── OPS — keeps the lights on ──────────────────────────────────────────
  { name: "dev-tracker",           category: "OPS",         color: "text-blue-400",
    tagline: "Session-resume reconciler",
    description: "Reconciles HexFlo task state against git history. Surfaces in-progress / blocked / next tasks. Spawns agents for ready work when asked.",
    whenToUse: "First thing on session start: 'where did we leave off?'" },
  { name: "status-monitor",        category: "OPS",         color: "text-blue-400",
    tagline: "Passive event-bus observer",
    description: "Subscribes to STDB events, formats progress, flags anomalies (heartbeat stale >45s, score drop >10%, token usage >80%). Read-only.",
    whenToUse: "Long-running multi-agent sessions. Side-channel 'keep an eye on the swarm'." },
];

// ── Types ────────────────────────────────────────────────────────────────────

interface SwarmTask { id: string; title: string; status: string; agentId?: string; agent_id?: string; }
interface Swarm {
  id: string;
  name?: string;
  status?: string;
  tasks?: SwarmTask[];
  projectId?: string;
  project_id?: string;
}
interface ProjectInfo { id: string; name: string; rootPath?: string; status?: string; }
interface DecisionItem {
  id: string; kind: string;
  severity: "CRITICAL" | "HIGH" | "MEDIUM" | "LOW";
  title: string; reason: string; ageSeconds: number;
  suggestedAction: string; link: string | null;
}
interface DecisionsResponse { items: DecisionItem[]; total: number; bySeverity: Record<string, number>; }
interface ImproverStatus { score?: number; mean_reward?: number; meanReward?: number; topHypothesis?: string; deadLetter?: number; }

// ── Helpers ──────────────────────────────────────────────────────────────────

function severityClass(s: string): string {
  switch (s) {
    case "CRITICAL": return "bg-red-900/40 text-red-300 border-red-700";
    case "HIGH":     return "bg-orange-900/40 text-orange-300 border-orange-700";
    case "MEDIUM":   return "bg-yellow-900/30 text-yellow-300 border-yellow-700";
    default:         return "bg-gray-800 text-gray-400 border-gray-700";
  }
}

function ageShort(seconds: number): string {
  if (!seconds || seconds <= 0) return "—";
  if (seconds < 60) return `${seconds}s`;
  const m = Math.floor(seconds / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
}

type LaneName = "Backlog" | "Ready" | "Doing" | "Done";
function lane(status: string): LaneName {
  switch (status) {
    case "in_progress": case "assigned": return "Doing";
    case "completed": case "done":       return "Done";
    case "blocked":                       return "Backlog";
    default:                              return "Ready";
  }
}

// Map a target lane back to a task `status` value the API understands.
function laneToStatus(lane: LaneName): string {
  switch (lane) {
    case "Backlog": return "blocked";
    case "Ready":   return "pending";
    case "Doing":   return "in_progress";
    case "Done":    return "completed";
  }
}

// Which lane→lane transitions can the operator drag manually? Agent-owned
// lanes (Doing claim, Done complete) are read-only — letting humans drag
// to "Done" makes the board lie about what shipped.
function canDrag(from: LaneName, to: LaneName): boolean {
  if (from === to) return false;
  // Operator decisions: park in backlog or pull back to ready
  if (to === "Backlog") return true;
  if (from === "Backlog" && to === "Ready") return true;
  // Done → Backlog allowed via the rule above (re-open as blocked).
  // Everything else (Ready→Doing, Doing→Done, Done→Ready, Done→Doing) is agent-owned.
  return false;
}

function dragRejectReason(from: LaneName, to: LaneName): string {
  if (from === to) return "";
  if (from === "Ready" && to === "Doing") return "Doing is agent-owned — workers claim it via `hex task assign` or @swarm-coordinator dispatch.";
  if (to === "Done") return "Done is agent-owned — only the worker can mark a task complete (it requires a result artifact).";
  if (from === "Done") return "Re-opening completed work is rare. If you need it back in flight, drag it to Backlog (blocked) and ask an agent to re-claim.";
  return "Not a valid manual transition.";
}

// ── Subcomponents ────────────────────────────────────────────────────────────

const TeamRail: Component<{
  onlineNames: () => Set<string>;
  onSelect: (name: string) => void;
  selected: () => string | null;
  /** Pool status keyed by role name. Drives the dot color + count badge. */
  poolByRole: () => Map<string, PoolStatus>;
  onScale: (role: string, count: number) => void;
}> = (props) => {
  const grouped = createMemo(() => {
    const cats: Record<string, Persona[]> = { PRODUCT: [], ENGINEERING: [], QUALITY: [], DESIGN: [], OPS: [] };
    for (const p of PERSONAS) cats[p.category].push(p);
    return cats;
  });

  // Pool indicator: dot color + optional count badge.
  //   red       → crash-loop (operator action needed)
  //   green     → desired>0 and alive>=desired (healthy)
  //   yellow⚡  → desired>0 but alive<desired (spawning/transient)
  //   amber     → paused (operator-stopped)
  //   green-soft→ online via @-mention dispatch (chat path, no pool)
  //   gray      → idle placeholder (default — nothing to worry about)
  const indicator = (p: Persona): { dot: string; badge: string | null; tip: string } => {
    const pool = props.poolByRole().get(p.name);
    const onlineFromChat = props.onlineNames().has(p.name);
    if (pool?.inCrashLoop) {
      return {
        dot: "bg-red-500",
        badge: `${pool.aliveCount}/${pool.desiredCount}`,
        tip: `crash-loop · ${pool.exitedCount} exits — clear the flag in the supervisor panel`,
      };
    }
    if (pool && pool.desiredCount > 0) {
      const healthy = pool.aliveCount >= pool.desiredCount;
      return {
        dot: pool.paused ? "bg-yellow-500" : (healthy ? "bg-green-500" : "bg-yellow-400 animate-pulse"),
        badge: `${pool.aliveCount}/${pool.desiredCount}`,
        tip: pool.paused ? "paused" : (healthy ? "active" : "spawning"),
      };
    }
    if (onlineFromChat) {
      return { dot: "bg-green-600", badge: null, tip: "online (chat dispatch)" };
    }
    return { dot: "bg-gray-700", badge: null, tip: "idle — click + to scale up" };
  };

  return (
    <aside class="w-72 border-r border-gray-800 bg-gray-950 overflow-y-auto px-3 py-4">
      <h2 class="text-[11px] font-bold uppercase tracking-wider text-gray-300 mb-3 px-1">Team</h2>
      <p class="text-[10px] text-gray-400 px-1 mb-3 leading-relaxed">
        Click a persona to chat. Dot color = pool state · hover for <span class="text-cyan-400 font-mono">+</span>/<span class="text-cyan-400 font-mono">−</span>.
      </p>
      <For each={Object.entries(grouped())}>
        {([cat, members]) => (
          <div class="mb-5">
            <div class="flex items-center justify-between px-1 mb-1.5">
              <span class="text-[10px] font-bold uppercase tracking-wider text-gray-400">{cat}</span>
              <span class="text-[10px] text-gray-400">{members.length}</span>
            </div>
            <ul class="space-y-0.5">
              <For each={members}>
                {(p) => {
                  const ind = createMemo(() => indicator(p));
                  const pool = createMemo(() => props.poolByRole().get(p.name));
                  return (
                    <li
                      onClick={() => props.onSelect(p.name)}
                      title={`${p.tagline}\n${ind().tip}`}
                      classList={{
                        "group flex items-start gap-2 px-2 py-1 rounded cursor-pointer text-xs transition": true,
                        "bg-gray-900 ring-1 ring-cyan-700/50": props.selected() === p.name,
                        "hover:bg-gray-900": props.selected() !== p.name,
                      }}
                    >
                      <span class={`h-1.5 w-1.5 rounded-full flex-shrink-0 mt-1.5 ${ind().dot}`} />
                      <div class="flex-1 min-w-0">
                        <div class="flex items-center gap-1.5">
                          <span class={`${p.color} truncate`}>{p.name}</span>
                          <Show when={ind().badge}>
                            <span class="text-[9px] font-mono text-gray-400">{ind().badge}</span>
                          </Show>
                        </div>
                        <div class="text-[10px] text-gray-400 truncate">{p.tagline}</div>
                      </div>
                      {/* Hover-reveal scale buttons. + always present; − only when desired>0. */}
                      <div class="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition flex-shrink-0">
                        <Show when={pool() && pool()!.desiredCount > 0}>
                          <button
                            onClick={(e) => { e.stopPropagation(); props.onScale(p.name, Math.max(0, pool()!.desiredCount - 1)); }}
                            class="text-[11px] text-gray-400 hover:text-cyan-400 px-1"
                            title="Scale down by 1"
                          >−</button>
                        </Show>
                        <button
                          onClick={(e) => { e.stopPropagation(); props.onScale(p.name, (pool()?.desiredCount ?? 0) + 1); }}
                          class="text-[11px] text-gray-400 hover:text-cyan-400 px-1"
                          title="Scale up by 1"
                        >+</button>
                      </div>
                    </li>
                  );
                }}
              </For>
            </ul>
          </div>
        )}
      </For>
    </aside>
  );
};

const KanbanLanes: Component<{
  swarms: () => Swarm[];
  onSendToChat: (text: string, role?: string) => void;
  /** Called after a successful drag-drop transition so the parent can refetch. */
  onTaskMoved?: () => void;
}> = (props) => {
  // Track the dragged task + its source lane so drops can validate the
  // transition and PATCH the right id.
  const [dragging, setDragging] = createSignal<{ taskId: string; from: LaneName } | null>(null);
  const [dragOverLane, setDragOverLane] = createSignal<LaneName | null>(null);
  const [rejectMsg, setRejectMsg] = createSignal<string>("");
  let rejectTimer: number | undefined;

  const showReject = (msg: string) => {
    setRejectMsg(msg);
    if (rejectTimer !== undefined) window.clearTimeout(rejectTimer);
    rejectTimer = window.setTimeout(() => setRejectMsg(""), 4000);
  };

  const handleDrop = async (toLane: LaneName) => {
    const d = dragging();
    setDragging(null);
    setDragOverLane(null);
    if (!d) return;
    if (!canDrag(d.from, toLane)) {
      showReject(dragRejectReason(d.from, toLane));
      return;
    }
    // PATCH /api/hexflo/tasks/{id} with the new status.
    try {
      await restClient.patch(`/api/hexflo/tasks/${d.taskId}`, {
        task_id: d.taskId,
        status: laneToStatus(toLane),
      });
      props.onTaskMoved?.();
    } catch (e) {
      showReject(`move failed: ${e instanceof Error ? e.message : String(e)}`);
    }
  };

  const tasksByLane = createMemo(() => {
    const lanes: Record<string, SwarmTask[]> = { Backlog: [], Ready: [], Doing: [], Done: [] };
    for (const s of props.swarms()) {
      for (const t of s.tasks || []) {
        const l = lane(t.status || "");
        if (lanes[l].length < 8) lanes[l].push(t);
      }
    }
    return lanes;
  });

  // Try to extract a role from the task title — many are "hex-coder: ..." or
  // a JSON object {"role":"X","description":"..."}. Falls back to swarm-coordinator.
  const taskRole = (t: SwarmTask): string => {
    try {
      const obj = JSON.parse(t.title);
      if (obj && typeof obj.role === "string") return obj.role;
    } catch { /* not JSON */ }
    const m = t.title.match(/^([\w-]+):\s/);
    if (m && PERSONAS.find((p) => p.name === m[1])) return m[1];
    return "swarm-coordinator";
  };

  const taskClick = (t: SwarmTask) => {
    const role = taskRole(t);
    const status = t.status || "?";
    let title = t.title;
    try {
      const obj = JSON.parse(title);
      title = obj.description || obj.title || title;
    } catch { /* ignore */ }
    const prompt = status === "completed" || status === "done"
      ? `@${role} can you summarize what you did for: ${title}`
      : `@${role} status check: ${title}\n\n(currently ${status})`;
    props.onSendToChat(prompt, role);
  };

  return (
    <section class="bg-gray-900/50 border border-gray-800 rounded-lg p-3 mb-4">
      <div class="flex items-center justify-between mb-1 px-1">
        <h3 class="text-xs font-bold uppercase tracking-wider text-gray-400">Kanban</h3>
        <Show when={rejectMsg()}>
          <span class="text-[10px] text-yellow-400" role="alert">⚠ {rejectMsg()}</span>
        </Show>
      </div>
      <p class="text-[10px] text-gray-400 px-1 mb-2">
        Click a card to chat · drag to move · ● assigned · ○ unclaimed · agent-owned lanes (Doing/Done) reject manual drops
      </p>
      <div class="grid grid-cols-4 gap-2">
        <For each={["Backlog", "Ready", "Doing", "Done"] as LaneName[]}>
          {(laneName) => {
            const isValidDrop = () => {
              const d = dragging();
              return d ? canDrag(d.from, laneName) : false;
            };
            const isDragOver = () => dragOverLane() === laneName;
            return (
              <div
                onDragOver={(e) => {
                  if (!dragging()) return;
                  e.preventDefault();
                  setDragOverLane(laneName);
                  e.dataTransfer!.dropEffect = isValidDrop() ? "move" : "none";
                }}
                onDragLeave={() => { if (dragOverLane() === laneName) setDragOverLane(null); }}
                onDrop={(e) => {
                  e.preventDefault();
                  handleDrop(laneName);
                }}
                class="bg-gray-950 border border-gray-800 rounded p-2 min-h-[120px] transition"
                classList={{
                  "ring-2 ring-cyan-600 border-cyan-700": isDragOver() && isValidDrop(),
                  "ring-2 ring-red-700/50 border-red-800/50": isDragOver() && !isValidDrop(),
                  "opacity-60": dragging() != null && !isValidDrop() && dragging()?.from !== laneName,
                }}
              >
                <div class="flex items-center justify-between mb-1.5">
                  <span class="text-[10px] font-bold uppercase tracking-wider text-gray-300">
                    {laneName}
                    <Show when={dragging() && !canDrag(dragging()!.from, laneName) && dragging()!.from !== laneName}>
                      <span class="text-gray-600 ml-1" title="agent-owned — drag rejected">🔒</span>
                    </Show>
                  </span>
                  <span class="text-[10px] text-gray-400">{tasksByLane()[laneName].length}</span>
                </div>
                <ul class="space-y-1">
                  <For each={tasksByLane()[laneName]}>
                    {(t) => {
                      const agentId = t.agentId || t.agent_id || "";
                      const dot = agentId ? "●" : "○";
                      const role = taskRole(t);
                      return (
                        <li
                          draggable={true}
                          onDragStart={(e) => {
                            setDragging({ taskId: t.id, from: laneName });
                            e.dataTransfer!.effectAllowed = "move";
                          }}
                          onDragEnd={() => { setDragging(null); setDragOverLane(null); }}
                          onClick={() => taskClick(t)}
                          class="text-[11px] text-gray-200 bg-gray-900 border border-gray-800 rounded px-2 py-1 truncate cursor-grab active:cursor-grabbing hover:border-cyan-700 hover:bg-gray-800 transition group"
                          classList={{ "opacity-50": dragging()?.taskId === t.id }}
                          title={`${t.title}\nRole: ${role}\n\nClick to chat · Drag to move`}
                        >
                          <span class="text-gray-400 mr-1">{dot}</span>
                          <span class="group-hover:text-gray-100">
                            {t.title.slice(0, 26)}{t.title.length > 26 ? "…" : ""}
                          </span>
                        </li>
                      );
                    }}
                  </For>
                  <Show when={tasksByLane()[laneName].length === 0}>
                    <li class="text-[10px] text-gray-300 italic px-2 py-2">empty</li>
                  </Show>
                </ul>
              </div>
            );
          }}
        </For>
      </div>
    </section>
  );
};

// Short, conversational prompts. The agent already has the project context
// + grounded facts injected, so we don't need to repeat the long reason text
// here — that's just visual noise in the chat history.
function decisionAction(item: DecisionItem): { role: string; prompt: string } {
  // Trim the title so the chat bubble is readable
  const t = item.title.length > 80 ? item.title.slice(0, 77) + "…" : item.title;
  switch (item.kind) {
    case "blocked_task":
      return { role: "pm-agent", prompt: `@pm-agent help me unblock: ${t}` };
    case "proposed_adr": {
      const adrName = item.title.replace(/^ADR aging in Proposed: /, "");
      return { role: "adr-reviewer", prompt: `@adr-reviewer should ${adrName} be accepted, superseded, or closed?` };
    }
    case "persona_bypass":
      return { role: "pm-agent", prompt: `@pm-agent ${t}` };
    case "priority_inbox":
      return { role: "pm-agent", prompt: `@pm-agent inbox: ${t}` };
    default:
      return { role: "pm-agent", prompt: `@pm-agent decision: ${t}` };
  }
}

const DecisionsPanel: Component<{
  data: () => DecisionsResponse | null;
  onSendToChat: (text: string, role?: string) => void;
}> = (props) => {
  // Expanded shows all decisions; collapsed shows top 5. Toggle in-place
  // (no navigation) so the operator stays in Brain.
  const [expanded, setExpanded] = createSignal(false);
  const visible = createMemo(() => {
    const items = props.data()?.items || [];
    return expanded() ? items : items.slice(0, 5);
  });
  return (
    <section class="bg-gray-900/50 border border-gray-800 rounded-lg p-3 mb-4">
      <div class="flex items-center justify-between mb-1 px-1">
        <h3 class="text-xs font-bold uppercase tracking-wider text-gray-400">Decisions Needed</h3>
        <Show when={props.data()}>
          {(d) => (
            <div class="flex gap-1.5">
              <For each={["CRITICAL", "HIGH", "MEDIUM"]}>
                {(s) => (
                  <Show when={(d().bySeverity[s] || 0) > 0}>
                    <span class={`text-[10px] font-bold px-1.5 py-0.5 rounded border ${severityClass(s)}`}>
                      {d().bySeverity[s]} {s[0]}
                    </span>
                  </Show>
                )}
              </For>
            </div>
          )}
        </Show>
      </div>
      <p class="text-[10px] text-gray-400 px-1 mb-2">
        Click a card to ask an agent about it · the chat input pre-fills with the right @-mention.
      </p>
      <Show
        when={(props.data()?.items || []).length > 0}
        fallback={
          <div class="text-center text-gray-300 text-xs py-3">
            ✓ caught up — no decisions pending
          </div>
        }
      >
        <ul
          class="space-y-1"
          classList={{ "max-h-[420px] overflow-y-auto pr-1": expanded() }}
        >
          <For each={visible()}>
            {(item) => (
              <li
                onClick={() => {
                  const a = decisionAction(item);
                  props.onSendToChat(a.prompt, a.role);
                }}
                class="flex items-start gap-2 text-xs px-2 py-1.5 rounded cursor-pointer hover:bg-gray-800/50 transition group"
                title={`Click to ask @${decisionAction(item).role} about this decision`}
              >
                <span class={`text-[10px] font-bold px-1.5 py-0.5 rounded border ${severityClass(item.severity)} flex-shrink-0`}>
                  {item.severity[0]}
                </span>
                <span class="text-gray-200 truncate flex-1" title={item.title}>{item.title}</span>
                <span class="text-[10px] text-gray-400 flex-shrink-0">{ageShort(item.ageSeconds)}</span>
                <span class="text-cyan-400 text-[12px] opacity-0 group-hover:opacity-100 transition">→</span>
              </li>
            )}
          </For>
        </ul>
        <Show when={(props.data()?.items.length || 0) > 5}>
          <button
            onClick={() => setExpanded(!expanded())}
            class="w-full text-[11px] text-cyan-400 hover:text-cyan-300 hover:bg-gray-800/50 text-center py-1.5 mt-1 rounded transition"
          >
            <Show when={expanded()} fallback={<>show all {props.data()?.total} ↓</>}>
              collapse to top 5 ↑
            </Show>
          </button>
        </Show>
      </Show>
    </section>
  );
};

const SwarmsPanel: Component<{
  swarms: () => Swarm[];
  projectName: (id: string) => string;
}> = (props) => (
  <section class="bg-gray-900/50 border border-gray-800 rounded-lg p-3 mb-4">
    <h3 class="text-xs font-bold uppercase tracking-wider text-gray-400 mb-2 px-1">Swarms</h3>
    <Show
      when={props.swarms().length > 0}
      fallback={<div class="text-xs text-gray-400 italic">no active swarms</div>}
    >
      <ul class="space-y-2">
        <For each={props.swarms().slice(0, 6)}>
          {(s) => {
            const tasks = s.tasks || [];
            const completed = tasks.filter((t) => t.status === "completed").length;
            const failed = tasks.filter((t) => t.status === "failed").length;
            const inProgress = tasks.filter(
              (t) => t.status === "in_progress" || t.status === "assigned",
            );
            const pending = tasks.filter((t) => t.status === "pending");
            // Surface what this swarm is doing right now: pick the
            // first in-progress task; if none, the next pending; else done.
            const focus = inProgress[0] || pending[0] || tasks[tasks.length - 1];
            // Distinct agents working any task in this swarm.
            const assigned = Array.from(
              new Set(
                tasks
                  .map((t) => (t.agentId || t.agent_id || "").toString())
                  .filter((a) => a && a !== "null"),
              ),
            );
            // Try to map agent IDs back to a known persona — best-effort, since
            // STDB stores UUIDs rather than role names. Fallback shows count.
            const focusTitle = focus?.title?.replace(/\s+/g, " ").trim() || "(no tasks)";
            return (
              <li
                class="text-xs bg-gray-950 border border-gray-800 rounded p-2 hover:border-gray-700 cursor-default"
                title={`Swarm: ${s.id}\nStatus: ${s.status || "active"}`}
              >
                <div class="flex items-center gap-2 mb-1">
                  <span
                    class={`h-1.5 w-1.5 rounded-full flex-shrink-0 ${
                      failed > 0 ? "bg-red-500" : inProgress.length > 0 ? "bg-green-500" : "bg-gray-600"
                    }`}
                  />
                  <span class="text-gray-200 font-medium truncate flex-1">{s.name || s.id.slice(0, 12)}</span>
                  {(() => {
                    const pid = (s.projectId || s.project_id || "").toString();
                    return pid ? (
                      <span
                        class="text-[10px] text-cyan-500 bg-cyan-900/20 px-1.5 py-0 rounded border border-cyan-900 flex-shrink-0"
                        title={`Project: ${props.projectName(pid)}`}
                      >
                        {props.projectName(pid)}
                      </span>
                    ) : (
                      <span class="text-[10px] text-gray-300 flex-shrink-0" title="No project — global swarm">global</span>
                    );
                  })()}
                  <span class="text-[10px] text-gray-300 flex-shrink-0 font-mono">
                    {completed}/{tasks.length}
                    {failed > 0 ? <span class="text-red-400 ml-1">· {failed} fail</span> : null}
                  </span>
                </div>
                <div class="text-[11px] text-gray-400 truncate pl-3.5" title={focusTitle}>
                  <span class="text-gray-400">→</span> {focusTitle.slice(0, 80)}
                </div>
                <div class="text-[10px] text-gray-400 pl-3.5 mt-0.5 flex gap-2">
                  <Show
                    when={assigned.length > 0}
                    fallback={<span>unassigned</span>}
                  >
                    <span>
                      agent{assigned.length > 1 ? "s" : ""}:{" "}
                      <span class="text-gray-300 font-mono">
                        {assigned.slice(0, 2).map((a) => a.slice(0, 8)).join(", ")}
                        {assigned.length > 2 ? ` +${assigned.length - 2}` : ""}
                      </span>
                    </span>
                  </Show>
                  <Show when={pending.length > 0}>
                    <span class="text-gray-300">· {pending.length} pending</span>
                  </Show>
                </div>
              </li>
            );
          }}
        </For>
      </ul>
    </Show>
  </section>
);

interface PoolStatus {
  id: string;
  role: string;
  desiredCount: number;
  aliveCount: number;
  exitedCount: number;
  restartStrategy: string;
  paused: boolean;
  inCrashLoop: boolean;
}

interface SupervisorEvent {
  id: number;
  ts: string;
  kind: string;
  poolId: string;
  workerId: string;
  payload: string;
  handled: boolean;
}

const SupervisorPanel: Component = () => {
  const [pools, setPools] = createSignal<PoolStatus[]>([]);
  const [loading, setLoading] = createSignal(true);

  const refresh = async () => {
    try {
      const resp = await restClient.get<{ pools: PoolStatus[] }>("/api/pools");
      setPools(resp.pools || []);
    } catch { /* nexus may be down or table not present */ }
    setLoading(false);
  };

  let pollHandle: number | undefined;
  onMount(() => {
    refresh();
    pollHandle = window.setInterval(refresh, 10000);
  });
  onCleanup(() => { if (pollHandle !== undefined) window.clearInterval(pollHandle); });

  const setPoolPaused = async (id: string, paused: boolean) => {
    try {
      await restClient.patch(`/api/pools/${id}/paused`, { paused });
      refresh();
    } catch { /* surface in toast later */ }
  };

  // Scale a pool: read existing config, post back with new desired_count.
  // Bumps to 1 when first activating from idle.
  const scalePool = async (p: PoolStatus, count: number) => {
    try {
      await restClient.post("/api/pools", {
        id: p.id,
        role: p.role,
        desired_count: count,
        restart_strategy: p.restartStrategy,
        max_restarts: 5,
        max_restart_window_secs: 60,
        paused: false,
        owner_agent_id: "operator",
      });
      refresh();
    } catch { /* surface in toast later */ }
  };

  // Split into three buckets: ACTIVE (desired>0 OR running), CRASH (any
  // in_crash_loop), IDLE (the auto-seeded placeholders the operator hasn't
  // touched). Each gets its own visual treatment so 24 idle placeholders
  // don't drown out the 1-2 pools the operator actually cares about.
  const active = createMemo(() => pools().filter((p) => p.desiredCount > 0 && !p.inCrashLoop));
  const crashed = createMemo(() => pools().filter((p) => p.inCrashLoop));
  const idle = createMemo(() => pools().filter((p) => p.desiredCount === 0 && !p.inCrashLoop));

  return (
    <section class="bg-gray-900/50 border border-gray-800 rounded-lg p-3 mb-4">
      <div class="flex items-center justify-between mb-2 px-1">
        <h3 class="text-xs font-bold uppercase tracking-wider text-gray-400">Worker Pools</h3>
        <span class="text-[10px] text-gray-400">
          <span class="text-green-400">{active().length} active</span>
          <span class="mx-1.5 text-gray-700">·</span>
          <span class="text-red-400">{crashed().length} crash</span>
          <span class="mx-1.5 text-gray-700">·</span>
          <span class="text-gray-500">{idle().length} idle</span>
        </span>
      </div>

      <Show when={loading()}>
        <div class="text-center text-gray-400 text-xs py-3">loading…</div>
      </Show>

      {/* CRASH section — surface first because it needs operator action. */}
      <Show when={crashed().length > 0}>
        <div class="mb-3">
          <div class="text-[10px] font-bold uppercase tracking-wider text-red-400 mb-1 px-1">⚠ Crash-looped</div>
          <ul class="space-y-1">
            <For each={crashed()}>
              {(p) => (
                <li class="grid grid-cols-[3rem_1fr_auto_auto] items-center gap-3 text-xs px-2 py-1.5 rounded bg-red-950/20 border border-red-900/40">
                  <span class="font-mono font-bold text-right text-red-400">{p.aliveCount}/{p.desiredCount}</span>
                  <span class="text-gray-100 truncate min-w-0" title={`${p.id}\nrole: ${p.role}\nexited: ${p.exitedCount}`}>
                    {p.id}
                    <span class="text-red-400/70 ml-2">· {p.exitedCount} exits</span>
                  </span>
                  <button
                    onClick={async () => {
                      await restClient.patch(`/api/pools/${p.id}/paused`, { paused: false });
                      await restClient.patch(`/api/pools/${p.id}/paused`, { paused: true });
                      refresh();
                    }}
                    class="text-[10px] text-red-300 hover:text-red-200 underline"
                    title="Clear crash-loop flag (resumes briefly to clear, then re-pauses)"
                  >
                    clear flag
                  </button>
                  <button
                    onClick={() => setPoolPaused(p.id, false)}
                    class="text-[10px] text-cyan-400 hover:text-cyan-300 underline"
                    title="Resume — clears crash flag and restarts spawning"
                  >
                    resume
                  </button>
                </li>
              )}
            </For>
          </ul>
        </div>
      </Show>

      {/* ACTIVE section — pools the operator has explicitly scaled up. */}
      <Show when={active().length > 0}>
        <div class="mb-3">
          <div class="text-[10px] font-bold uppercase tracking-wider text-green-400 mb-1 px-1">Active</div>
          <ul class="space-y-1">
            <For each={active()}>
              {(p) => {
                const healthy = p.aliveCount >= p.desiredCount;
                return (
                  <li class="grid grid-cols-[3rem_1fr_auto_auto_auto] items-center gap-3 text-xs px-2 py-1.5 rounded bg-gray-950 border border-gray-800">
                    <span class={`font-mono font-bold text-right ${healthy ? "text-green-400" : "text-yellow-400"}`}>
                      {p.aliveCount}/{p.desiredCount}
                    </span>
                    <span class="text-gray-200 truncate min-w-0" title={`${p.id}\nrole: ${p.role}\nstrategy: ${p.restartStrategy}\nexits: ${p.exitedCount}`}>
                      {p.id}
                      <span class="text-gray-500 ml-2">· {p.role}</span>
                    </span>
                    <Show when={p.paused}>
                      <span class="text-[10px] font-bold px-1.5 py-0.5 rounded border bg-yellow-900/30 text-yellow-300 border-yellow-700">paused</span>
                    </Show>
                    <button
                      onClick={() => scalePool(p, Math.max(0, p.desiredCount - 1))}
                      class="text-[11px] text-gray-400 hover:text-cyan-400 px-1"
                      title="Scale down by 1"
                    >
                      −
                    </button>
                    <button
                      onClick={() => scalePool(p, p.desiredCount + 1)}
                      class="text-[11px] text-gray-400 hover:text-cyan-400 px-1"
                      title="Scale up by 1"
                    >
                      +
                    </button>
                  </li>
                );
              }}
            </For>
          </ul>
        </div>
      </Show>

      {/* IDLE section — collapsed by default. Click to expand and scale up. */}
      <Show when={idle().length > 0}>
        <details class="text-xs">
          <summary class="cursor-pointer text-[10px] font-bold uppercase tracking-wider text-gray-400 mb-1 px-1 hover:text-gray-200">
            ▸ Available roles ({idle().length}) — click to scale up
          </summary>
          <div class="grid grid-cols-3 gap-1 mt-2">
            <For each={idle()}>
              {(p) => (
                <button
                  onClick={() => scalePool(p, 1)}
                  class="text-[10px] text-left px-2 py-1 rounded border border-gray-800 bg-gray-950 hover:border-cyan-700 hover:bg-gray-900 transition group"
                  title={`Scale ${p.role} to 1 worker`}
                >
                  <span class="text-gray-300 group-hover:text-cyan-300 truncate block">{p.role}</span>
                </button>
              )}
            </For>
          </div>
        </details>
      </Show>

      <Show when={!loading() && pools().length === 0}>
        <div class="text-center text-gray-300 text-xs py-3">
          no pools defined — <code class="text-cyan-400">hex pool create</code>
        </div>
      </Show>
    </section>
  );
};

const HealthPanel: Component<{ improver: () => ImproverStatus | null; swarmCount: () => number }> = (props) => (
  <section class="bg-gray-900/50 border border-gray-800 rounded-lg p-3">
    <h3 class="text-xs font-bold uppercase tracking-wider text-gray-400 mb-2 px-1">Health</h3>
    <dl class="grid grid-cols-2 gap-y-1.5 gap-x-4 text-xs">
      <dt class="text-gray-300">Homeostasis</dt>
      <dd class="text-gray-200 font-mono text-right">
        {props.improver()?.score ?? "—"}
        <Show when={props.improver()?.score !== undefined && (props.improver()?.score ?? 0) > 50}>
          <span class="text-green-400 ml-1">↗</span>
        </Show>
      </dd>
      <dt class="text-gray-300">Q-reward</dt>
      <dd class="text-gray-200 font-mono text-right">
        {(() => {
          const r = props.improver()?.mean_reward ?? props.improver()?.meanReward;
          return r === undefined ? "—" : (r >= 0 ? "+" : "") + r.toFixed(3);
        })()}
      </dd>
      <dt class="text-gray-300">Active swarms</dt>
      <dd class="text-gray-200 font-mono text-right">{props.swarmCount()}</dd>
      <dt class="text-gray-300">Dead-letter</dt>
      <dd class="text-gray-200 font-mono text-right">{props.improver()?.deadLetter ?? "—"}</dd>
    </dl>
  </section>
);

interface ChatMessage {
  from: "you" | string; // "you" or persona name
  text: string;
  ts: string;
  model?: string;
  pending?: boolean;
  error?: boolean;
  /** Performance.now() timestamp at dispatch start; used to render elapsed seconds in pending bubbles. */
  startedAt?: number;
}

// Parse "@<role> <message>" — returns { role, message } or null if no @-mention.
function parseAtMention(text: string): { role: string; message: string } | null {
  const m = text.match(/^@([\w-]+)\s+([\s\S]+)$/);
  if (!m) return null;
  return { role: m[1], message: m[2].trim() };
}

// Resolve a broadcast target like "all", "product", "engineering" to a list
// of persona names. Returns null if it's not a broadcast keyword.
function resolveBroadcastTarget(token: string): string[] | null {
  const t = token.toLowerCase();
  if (t === "all" || t === "team") return PERSONAS.map((p) => p.name);
  const categoryMap: Record<string, "PRODUCT" | "ENGINEERING" | "QUALITY" | "DESIGN" | "OPS"> = {
    product: "PRODUCT",
    engineering: "ENGINEERING",
    eng: "ENGINEERING",
    quality: "QUALITY",
    qa: "QUALITY",
    design: "DESIGN",
    ops: "OPS",
  };
  const cat = categoryMap[t];
  if (!cat) return null;
  return PERSONAS.filter((p) => p.category === cat).map((p) => p.name);
}

// Chat threads are now persisted to STDB via /api/brain/threads (each thread
// is a hexflo_memory entry under key "chat:thread:<uuid>"). The "active
// thread id" lives in localStorage so a refresh resumes the same conversation;
// the messages themselves are loaded from the server.
const ACTIVE_THREAD_KEY = "hex-brain-chat-active-thread-v2";

const WELCOME: ChatMessage = {
  from: "system",
  text: "Type @<role> <message> to talk to one agent. Follow-ups without @ continue with the same agent. Broadcast with @all, @product, @engineering, @quality, @design, @ops. Threads persist in SpacetimeDB — open from any browser/machine.",
  ts: new Date().toISOString(),
};

interface ThreadSummary {
  id: string;
  title: string;
  projectId?: string;
  createdAt: string;
  lastActiveAt: string;
  messageCount?: number;
}

// localStorage key for the chat-panel width — separate from history so a
// user can clear chat without losing their layout.
const CHAT_WIDTH_KEY = "hex-brain-chat-width-v1";
const CHAT_WIDTH_MIN = 280;
const CHAT_WIDTH_MAX = 900;
const CHAT_WIDTH_DEFAULT = 384;

const ChatPanel: Component<{
  selectedAgent: () => string | null;
  onAgentChange: (name: string | null) => void;
  /** Active project ID — passed to /api/brain/chat so the persona's system
      prompt is prefixed with PROJECT CONTEXT (name + rootPath). Empty string
      or "__global__" means no project scoping. */
  projectId: () => string;
  /** Display name of the active project for the chat header. */
  projectName: () => string;
  /** Externally-driven input value — lets DecisionsPanel / KanbanLanes
      pre-fill the textarea with a click-to-ask prompt. */
  externalInput: () => string;
  setExternalInput: (text: string) => void;
  /** Bumped by Brain when a card click should auto-send the prefilled input
      instead of waiting for Cmd+Enter. */
  autoSendBump: () => number;
}> = (props) => {
  // Resizable width — drag the left edge of the chat pane to resize. Persists
  // to localStorage so the choice survives refresh.
  const [width, setWidth] = createSignal<number>(
    (() => {
      const raw = localStorage.getItem(CHAT_WIDTH_KEY);
      const n = raw ? parseInt(raw, 10) : NaN;
      return Number.isFinite(n) && n >= CHAT_WIDTH_MIN && n <= CHAT_WIDTH_MAX
        ? n
        : CHAT_WIDTH_DEFAULT;
    })(),
  );
  const [resizing, setResizing] = createSignal(false);

  // Mouse-driven resize — listeners attach on mousedown and detach on mouseup.
  // Computes new width from window.innerWidth - mouseX so the drag tracks the
  // RIGHT panel's left edge regardless of left-rail / center-content widths.
  const handleResizeMouseDown = (e: MouseEvent) => {
    e.preventDefault();
    setResizing(true);
    const onMove = (ev: MouseEvent) => {
      const next = Math.min(
        CHAT_WIDTH_MAX,
        Math.max(CHAT_WIDTH_MIN, window.innerWidth - ev.clientX),
      );
      setWidth(next);
    };
    const onUp = () => {
      setResizing(false);
      try { localStorage.setItem(CHAT_WIDTH_KEY, String(width())); } catch { /* quota */ }
      window.removeEventListener("mousemove", onMove);
      window.removeEventListener("mouseup", onUp);
    };
    window.addEventListener("mousemove", onMove);
    window.addEventListener("mouseup", onUp);
  };

  // Input is externally drivable — Brain owns the signal so other panels
  // can pre-fill it on card click. Local input read/write goes through props.
  const input = props.externalInput;
  const setInput = props.setExternalInput;
  const [history, setHistory] = createSignal<ChatMessage[]>([WELCOME]);
  // Active thread id — persisted in localStorage so a refresh resumes the
  // same conversation. Messages themselves live in STDB.
  const [activeThreadId, setActiveThreadId] = createSignal<string | null>(
    localStorage.getItem(ACTIVE_THREAD_KEY),
  );
  const [threads, setThreads] = createSignal<ThreadSummary[]>([]);
  const [showThreads, setShowThreads] = createSignal(false);

  // Ensure a thread exists. Returns the thread id; creates one lazily on
  // first message so empty browsing doesn't pollute STDB with empty threads.
  const ensureThread = async (): Promise<string> => {
    const existing = activeThreadId();
    if (existing) return existing;
    const resp = await restClient.post<{ id: string }>("/api/brain/threads", {
      title: "new thread",
      project_id: props.projectId() || undefined,
    });
    setActiveThreadId(resp.id);
    localStorage.setItem(ACTIVE_THREAD_KEY, resp.id);
    refreshThreads();
    return resp.id;
  };

  // Append one message to STDB without blocking the UI. Errors are swallowed
  // (the user already sees the message in their local history).
  const persistMessage = async (msg: ChatMessage) => {
    if (msg.pending) return;
    try {
      const tid = await ensureThread();
      await restClient.post(`/api/brain/threads/${tid}/messages`, {
        message: {
          from: msg.from,
          text: msg.text,
          ts: msg.ts,
          model: msg.model,
          error: msg.error,
        },
      });
    } catch { /* surface later if it becomes a recurring issue */ }
  };

  const refreshThreads = async () => {
    try {
      const resp = await restClient.get<{ threads: ThreadSummary[] }>("/api/brain/threads");
      setThreads(resp.threads || []);
    } catch { /* nexus may be down */ }
  };

  const loadThread = async (id: string) => {
    try {
      const resp = await restClient.get<{ messages?: ChatMessage[] }>(`/api/brain/threads/${id}`);
      const msgs = resp.messages || [];
      setHistory(msgs.length > 0 ? msgs : [WELCOME]);
      setActiveThreadId(id);
      localStorage.setItem(ACTIVE_THREAD_KEY, id);
      setShowThreads(false);
    } catch { /* thread missing — clear and start fresh */ }
  };

  const newThread = async () => {
    const resp = await restClient.post<{ id: string }>("/api/brain/threads", {
      title: "new thread",
      project_id: props.projectId() || undefined,
    });
    setActiveThreadId(resp.id);
    localStorage.setItem(ACTIVE_THREAD_KEY, resp.id);
    setHistory([WELCOME]);
    refreshThreads();
    setShowThreads(false);
  };

  // On mount: if we have an active thread id, load its messages from STDB.
  // Always refresh the threads list so the picker has data.
  onMount(() => {
    refreshThreads();
    const tid = activeThreadId();
    if (tid) loadThread(tid);
  });
  const [showSuggestions, setShowSuggestions] = createSignal(false);
  const [suggestionQuery, setSuggestionQuery] = createSignal("");
  const [tick, setTick] = createSignal(0);
  // Active agent — driven by props (so TeamRail clicks stay in sync) but
  // also writable from inside the chat (parsing @-mention, clear button).
  const currentAgent = props.selectedAgent;
  const setCurrentAgent = (name: string | null) => props.onAgentChange(name);
  // Look up the persona record for description display.
  const currentPersona = createMemo(() => {
    const name = currentAgent();
    return name ? PERSONAS.find((p) => p.name === name) ?? null : null;
  });

  // Tick once per second so any pending bubbles re-render their elapsed counter.
  // Cheap — only affects the chat pane.
  let tickHandle: number | undefined;
  onMount(() => { tickHandle = window.setInterval(() => setTick((t) => t + 1), 1000); });
  onCleanup(() => { if (tickHandle !== undefined) window.clearInterval(tickHandle); });

  // Synthetic "broadcast persona" entries for autocomplete only — they aren't
  // real agents. Keep this list in sync with resolveBroadcastTarget keywords.
  const BROADCAST_TOKENS: { name: string; category: string; color: string; tagline: string }[] = [
    { name: "all",         category: "BROADCAST", color: "text-fuchsia-400", tagline: "all 25 personas in parallel" },
    { name: "product",     category: "BROADCAST", color: "text-purple-400",  tagline: "5 PRODUCT agents (planning + specs)" },
    { name: "engineering", category: "BROADCAST", color: "text-green-400",   tagline: "8 ENGINEERING agents (codegen)" },
    { name: "quality",     category: "BROADCAST", color: "text-red-400",     tagline: "7 QUALITY agents (review + judge)" },
    { name: "design",      category: "BROADCAST", color: "text-pink-400",    tagline: "2 DESIGN agents (CLI + UX critique)" },
    { name: "ops",         category: "BROADCAST", color: "text-blue-400",    tagline: "2 OPS agents (resume + monitor)" },
  ];

  // Filter personas for @-mention autocomplete. Includes broadcast tokens at top.
  const suggestions = createMemo(() => {
    const q = suggestionQuery().toLowerCase();
    const broadcasts = BROADCAST_TOKENS.filter((b) => b.name.toLowerCase().startsWith(q));
    const personas = PERSONAS.filter((p) => p.name.toLowerCase().startsWith(q));
    return [...broadcasts, ...personas].slice(0, 10);
  });

  const handleInput = (val: string) => {
    setInput(val);
    // Show suggestions while the user is mid-@-mention (cursor right after @<chars>).
    const m = val.match(/(?:^|\s)@([\w-]*)$/);
    if (m) {
      setSuggestionQuery(m[1]);
      setShowSuggestions(true);
    } else {
      setShowSuggestions(false);
    }
  };

  const completeSuggestion = (name: string) => {
    const cur = input();
    const replaced = cur.replace(/(?:^|\s)@([\w-]*)$/, (m, _q, _o, _s) => {
      // Preserve the leading whitespace if any.
      const leadingSpace = m.startsWith(" ") || m.startsWith("\n") ? m[0] : "";
      return `${leadingSpace}@${name} `;
    });
    setInput(replaced);
    setShowSuggestions(false);
  };

  // When the auto-send token bumps, dispatch the current input. Skip the
  // first effect run on mount — only react to subsequent bumps.
  let lastBump = props.autoSendBump();
  createEffect(() => {
    const b = props.autoSendBump();
    if (b !== lastBump) {
      lastBump = b;
      // Defer one tick so the input signal value lands before send.
      queueMicrotask(() => handleSend());
    }
  });

  // Auto-scroll the messages container to the bottom when new messages land.
  // Smart behavior: only auto-scroll if the user is already near the bottom
  // (within 80px). If they've scrolled up to read history, don't yank them
  // back — they're reading on purpose.
  let scrollContainer: HTMLDivElement | undefined;
  let stickToBottom = true;
  const onScroll = () => {
    if (!scrollContainer) return;
    const distanceFromBottom = scrollContainer.scrollHeight
      - scrollContainer.scrollTop
      - scrollContainer.clientHeight;
    stickToBottom = distanceFromBottom < 80;
  };
  createEffect(() => {
    history();  // re-run whenever messages change
    if (!scrollContainer || !stickToBottom) return;
    queueMicrotask(() => {
      if (scrollContainer) scrollContainer.scrollTop = scrollContainer.scrollHeight;
    });
  });

  const handleSend = async () => {
    const text = input().trim();
    if (!text) return;
    const ts = new Date().toISOString();

    // Resolve target agent:
    //   1. @all / @product / @engineering / @quality / @design / @ops → broadcast
    //   2. Explicit @<role> → single dispatch, switches currentAgent
    //   3. No @-mention but currentAgent is set → continue with that agent
    //   4. Neither → instructional error
    const parsed = parseAtMention(text);
    let targetRole: string | null = null;
    let messageBody = text;
    let broadcastTargets: string[] | null = null;
    if (parsed) {
      const broadcast = resolveBroadcastTarget(parsed.role);
      if (broadcast) {
        broadcastTargets = broadcast;
        messageBody = parsed.message;
      } else {
        targetRole = parsed.role;
        messageBody = parsed.message;
        setCurrentAgent(parsed.role);
      }
    } else if (currentAgent()) {
      targetRole = currentAgent();
    }

    const userMsg: ChatMessage = { from: "you", text, ts };
    setHistory((h) => [...h, userMsg]);
    setInput("");
    setShowSuggestions(false);
    persistMessage(userMsg);

    // Resolve project context to send to the backend. Empty string means
    // "no scope"; backend falls back to nexus cwd for context-free dispatch.
    const projectId = props.projectId() || "";

    // Broadcast path — fan out, render each response as its own bubble.
    if (broadcastTargets) {
      const groupId = Math.random().toString(36).slice(2);
      const startedAt = performance.now();
      // Single pending bubble representing the in-flight broadcast; replaced
      // with N bubbles when responses land.
      setHistory((h) => [
        ...h,
        {
          from: "broadcast",
          text: `dispatching to ${broadcastTargets!.length} personas...`,
          ts: new Date().toISOString(),
          pending: true,
          model: groupId,
          startedAt,
        },
      ]);
      try {
        const resp = await restClient.post<{
          message: string;
          responses: { role: string; model?: string; content?: string; error?: string }[];
          total: number;
        }>("/api/brain/broadcast", {
          message: messageBody,
          roles: broadcastTargets,
          project_id: projectId || undefined,
        });
        const bubbles: ChatMessage[] = resp.responses.map((r) => ({
          from: r.role,
          text: r.error ? `error: ${r.error}` : (r.content || "(empty response)"),
          ts: new Date().toISOString(),
          model: r.model,
          error: !!r.error,
        }));
        // Replace the pending bubble with N new bubbles, one per response.
        setHistory((h) => {
          const idx = h.findIndex((m) => m.pending && m.model === groupId);
          if (idx === -1) return h;
          const before = h.slice(0, idx);
          const after = h.slice(idx + 1);
          return [...before, ...bubbles, ...after];
        });
        // Persist each one to STDB. Done sequentially with await; user already
        // sees the bubbles, so latency is tolerable.
        for (const b of bubbles) await persistMessage(b);
      } catch (e) {
        const err = e instanceof Error ? e.message : String(e);
        setHistory((h) => h.map((m) =>
          m.pending && m.model === groupId
            ? { from: "broadcast", text: `broadcast failed: ${err}`, ts: new Date().toISOString(), error: true }
            : m,
        ));
      }
      return;
    }

    if (!targetRole) {
      setHistory((h) => [
        ...h,
        {
          from: "system",
          text: "Start your message with @<role> — e.g. `@pm-agent ...`. After that, follow-up messages without @ continue with the same agent.",
          ts: new Date().toISOString(),
          error: true,
        },
      ]);
      return;
    }

    // Optimistic pending bubble. Track startedAt so the bubble can show
    // elapsed seconds — local Ollama can take 30-60s on first generation.
    const pendingId = Math.random().toString(36).slice(2);
    const startedAt = performance.now();
    setHistory((h) => [
      ...h,
      { from: targetRole!, text: "thinking...", ts: new Date().toISOString(), pending: true, model: pendingId, startedAt },
    ]);

    try {
      interface ChildResp {
        role: string;
        model?: string;
        content?: string;
        error?: unknown;
        children?: ChildResp[];
      }
      const resp = await restClient.post<{ role: string; model: string; content: string; children?: ChildResp[] }>(
        "/api/brain/chat",
        {
          role: targetRole,
          message: messageBody,
          project_id: projectId || undefined,
          thread_id: activeThreadId() || undefined,
        },
      );
      const finalMsg: ChatMessage = {
        from: resp.role,
        text: resp.content || "(empty response)",
        ts: new Date().toISOString(),
        model: resp.model,
      };
      // Replace the pending bubble with the real response.
      setHistory((h) => h.map((m) =>
        m.pending && m.model === pendingId ? finalMsg : m,
      ));
      persistMessage(finalMsg);

      // Recursively flatten children into the chat history. Each child is
      // an auto-dispatch from a parent's @<role> mention; we render them as
      // ordinary chat bubbles, prefixed with "↳" so the threading is visible
      // without requiring nested DOM. The brief lives in the parent message,
      // not duplicated here — that's why text starts with the bare reply.
      const renderChildren = (children: ChildResp[] | undefined, prefix: string) => {
        for (const c of children ?? []) {
          const text = typeof c.content === "string" && c.content.length > 0
            ? c.content
            : (c.error ? `error: ${typeof c.error === "string" ? c.error : JSON.stringify(c.error)}` : "(empty response)");
          const childMsg: ChatMessage = {
            from: c.role,
            text: `${prefix} ${text}`,
            ts: new Date().toISOString(),
            model: typeof c.model === "string" ? c.model : undefined,
          };
          setHistory((h) => [...h, childMsg]);
          persistMessage(childMsg);
          renderChildren(c.children, `${prefix}↳`);
        }
      };
      renderChildren(resp.children, "↳");
    } catch (e) {
      const err = e instanceof Error ? e.message : String(e);
      setHistory((h) => h.map((m) =>
        m.pending && m.model === pendingId
          ? { from: targetRole!, text: `dispatch failed: ${err}`, ts: new Date().toISOString(), error: true }
          : m,
      ));
    }
  };

  const personaColor = (name: string): string => {
    const p = PERSONAS.find((x) => x.name === name);
    return p?.color ?? "text-gray-300";
  };

  return (
    <aside
      class="border-l border-gray-800 bg-gray-950 flex flex-col relative shrink-0"
      style={{ width: `${width()}px` }}
    >
      {/* Resize handle — 6px wide grab strip on the LEFT edge. Wider than the
          1px border so it's actually grabbable, with a hover/active highlight.
          Positioned -3px to overlap the border so the cursor changes earlier. */}
      <div
        onMouseDown={handleResizeMouseDown}
        class="absolute left-0 top-0 bottom-0 w-1.5 -ml-0.5 cursor-col-resize z-20 group"
        title="Drag to resize chat (280–900px)"
      >
        <div
          class="h-full w-px mx-auto bg-gray-800 group-hover:bg-cyan-600 transition-colors"
          classList={{ "!bg-cyan-500": resizing() }}
        />
      </div>
      <header class="px-3 py-2 border-b border-gray-800 relative">
        <div class="flex items-center justify-between mb-1">
          <button
            onClick={() => { setShowThreads(!showThreads()); refreshThreads(); }}
            class="text-[11px] font-bold uppercase tracking-wider text-gray-300 hover:text-gray-300 flex items-center gap-1.5"
            title="Click to switch thread"
          >
            <span>Chat</span>
            <span class="text-[10px] text-gray-300 font-normal normal-case tracking-normal">
              ▾ {threads().length} thread{threads().length === 1 ? "" : "s"}
            </span>
          </button>
          <div class="flex items-center gap-2 text-[10px] text-gray-400">
            <span title="Stored in SpacetimeDB · accessible from any browser/machine">
              ☁ {history().filter((m) => !m.pending).length}
            </span>
            <button
              onClick={newThread}
              class="text-gray-300 hover:text-cyan-400 underline"
              title="Start a fresh thread"
            >
              new
            </button>
          </div>
        </div>
        <Show when={showThreads()}>
          <ul class="absolute top-full left-0 right-0 mt-0 bg-gray-900 border border-gray-700 rounded-b shadow-xl max-h-72 overflow-y-auto z-30">
            <Show when={threads().length === 0}>
              <li class="px-3 py-2 text-[11px] text-gray-400 italic">no threads yet</li>
            </Show>
            <For each={threads()}>
              {(t) => {
                const active = t.id === activeThreadId();
                return (
                  <li
                    class={`px-3 py-1.5 text-xs cursor-pointer flex items-center justify-between gap-2 ${
                      active ? "bg-gray-800 border-l-2 border-cyan-500" : "hover:bg-gray-800"
                    }`}
                    onClick={() => loadThread(t.id)}
                  >
                    <div class="flex-1 min-w-0">
                      <div class="text-gray-200 truncate">{t.title || t.id.slice(0, 8)}</div>
                      <div class="text-[10px] text-gray-400">
                        {t.messageCount ?? 0} msg · {new Date(t.lastActiveAt).toLocaleString()}
                      </div>
                    </div>
                    <button
                      onClick={async (e) => {
                        e.stopPropagation();
                        if (!confirm("Delete this thread?")) return;
                        await fetch(`/api/brain/threads/${t.id}`, { method: "DELETE" });
                        if (active) {
                          setActiveThreadId(null);
                          localStorage.removeItem(ACTIVE_THREAD_KEY);
                          setHistory([WELCOME]);
                        }
                        refreshThreads();
                      }}
                      class="text-[10px] text-gray-300 hover:text-red-400"
                      title="Delete thread"
                    >
                      ✕
                    </button>
                  </li>
                );
              }}
            </For>
          </ul>
        </Show>
        <div
          class="text-[10px] flex items-center gap-1.5"
          title="All chat dispatches inject this project's PROJECT CONTEXT (name + rootPath) into the persona's system prompt. Change via the project picker in the top bar."
        >
          <Show
            when={props.projectId() && props.projectId() !== "__global__"}
            fallback={
              <>
                <span class="text-gray-300">⌖</span>
                <span class="text-gray-400">no project scope · responses are generic</span>
              </>
            }
          >
            <span class="text-cyan-500">⌖</span>
            <span class="text-gray-300">scoped to</span>
            <span class="text-cyan-400 font-medium">{props.projectName()}</span>
          </Show>
        </div>
      </header>
      <div
        ref={scrollContainer}
        onScroll={onScroll}
        class="flex-1 overflow-y-auto px-3 py-3 space-y-3"
      >
        <For each={history()}>
          {(msg) => (
            <div class="text-xs">
              <div class="text-[10px] mb-0.5 flex items-center gap-2">
                <span class={msg.from === "you" ? "text-gray-400" : personaColor(msg.from)}>
                  {msg.from === "you" ? "you" : `@${msg.from}`}
                </span>
                <span class="text-gray-400">· {new Date(msg.ts).toLocaleTimeString()}</span>
                <Show when={msg.model && !msg.pending}>
                  <span class="text-[9px] text-gray-300 ml-auto truncate max-w-[120px]" title={msg.model}>
                    {msg.model}
                  </span>
                </Show>
              </div>
              <Show
                when={msg.pending && msg.startedAt}
                fallback={
                  // Render agent responses as markdown — they emit ##
                  // headings, tables, code blocks, lists. "You" messages
                  // and errors stay as plain text to preserve @-mention
                  // syntax / show errors literally.
                  msg.from === "you" || msg.error || msg.from === "system" ? (
                    <div
                      class={`leading-relaxed whitespace-pre-wrap ${
                        msg.error ? "text-red-400" : "text-gray-300"
                      }`}
                    >
                      {msg.text}
                    </div>
                  ) : (
                    <MarkdownContent content={msg.text} />
                  )
                }
              >
                {(() => {
                  // Reference tick() so this re-runs each second.
                  tick();
                  const elapsed = Math.floor((performance.now() - msg.startedAt!) / 1000);
                  const hint = msg.from === "broadcast"
                    ? "cloud parallel — slowest of N determines wait"
                    : "cloud frontier ~2-10s · local Ollama 30-60s";
                  return (
                    <div class="leading-relaxed text-gray-300 italic">
                      thinking... ({elapsed}s — {hint})
                    </div>
                  );
                })()}
              </Show>
            </div>
          )}
        </For>
      </div>
      {/* Persona description card — only shown on a fresh/empty thread.
          Once the conversation has actual exchanges (any non-system messages),
          this disappears so the chat doesn't get re-cluttered after every reply. */}
      <Show when={currentPersona() && history().filter((m) => m.from !== "system").length === 0}>
        {(p) => (
          <div class="border-t border-gray-800 px-3 py-2 bg-gray-900/40">
            <div class="flex items-start justify-between gap-2 mb-1">
              <span class={`text-xs font-semibold ${p().color}`}>@{p().name}</span>
              <button
                onClick={() => setCurrentAgent(null)}
                class="text-[10px] text-gray-400 hover:text-gray-200 underline shrink-0"
              >
                clear
              </button>
            </div>
            <p class="text-[11px] text-gray-200 leading-relaxed mb-1">{p().description}</p>
            <p class="text-[10px] text-gray-400 leading-relaxed">
              <span class="text-gray-300 font-semibold uppercase tracking-wider">when to use</span> · {p().whenToUse}
            </p>
          </div>
        )}
      </Show>
      <footer class="border-t border-gray-800 p-2 relative">
        <Show when={showSuggestions() && suggestions().length > 0}>
          <ul class="absolute bottom-full left-2 right-2 mb-1 bg-gray-900 border border-gray-700 rounded shadow-xl max-h-48 overflow-y-auto z-10">
            <For each={suggestions()}>
              {(p) => (
                <li
                  class="px-2 py-1 text-xs hover:bg-gray-800 cursor-pointer flex items-center gap-2"
                  onClick={() => completeSuggestion(p.name)}
                >
                  <span class="text-[10px] text-gray-400 w-16">{p.category}</span>
                  <span class={p.color}>{p.name}</span>
                </li>
              )}
            </For>
          </ul>
        </Show>
        <textarea
          value={input()}
          onInput={(e) => handleInput(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              handleSend();
            } else if (e.key === "Escape") {
              setShowSuggestions(false);
            }
          }}
          placeholder={currentAgent() ? `reply to @${currentAgent()}...` : "@pm-agent classify..."}
          rows={3}
          class="w-full bg-gray-900 border border-gray-800 rounded px-2 py-1.5 text-xs text-gray-200 placeholder-gray-600 focus:outline-none focus:border-cyan-700 resize-none"
        />
        <div class="flex items-center justify-between mt-1.5">
          <span class="text-[10px] text-gray-400">
            {currentAgent() ? "Cmd+Enter to send · @ to switch agent" : "@ to mention · Cmd+Enter to send"}
          </span>
          <button
            onClick={handleSend}
            disabled={!input().trim()}
            class="px-2 py-1 text-[11px] font-medium bg-cyan-900/30 text-cyan-300 border border-cyan-700 rounded hover:bg-cyan-900/50 transition disabled:opacity-30 disabled:cursor-not-allowed"
          >
            Send
          </button>
        </div>
      </footer>
    </aside>
  );
};

// Synthesized high-signal event — derived by diffing successive polls of
// state we already fetch (swarms/tasks, decisions, agents). No new backend
// endpoint required — events are computed at the moment a poll lands.
interface DerivedEvent {
  ts: number;
  time: string;
  icon: string;
  color: string;
  source: string;
  text: string;
}

// Module-level event buffer + diff state. Survives component remounts within
// a session; resets on full page reload.
const eventBuffer: DerivedEvent[] = [];
let prevTaskStatus: Map<string, string> = new Map();
let prevSwarmIds: Set<string> = new Set();
let prevDecisionTotal = -1;
let prevAgentOnlineCount = -1;
// Pool deltas — alive_count / paused / in_crash_loop transitions.
const prevPoolState: Map<string, { alive: number; desired: number; paused: boolean; crash: boolean }> = new Map();
// Supervisor event log — last seen ID so we don't re-emit events on every poll.
let lastSupervisorEventId = 0;

function pushEvent(e: DerivedEvent) {
  eventBuffer.push(e);
  if (eventBuffer.length > 40) eventBuffer.splice(0, eventBuffer.length - 40);
}

function deriveEvents(args: {
  swarms: Swarm[];
  decisions: DecisionsResponse | null;
  agents: { name?: string; status?: string }[];
  pools?: PoolStatus[];
  supervisorEvents?: SupervisorEvent[];
}) {
  const now = performance.now();
  const time = new Date().toLocaleTimeString().slice(0, 8);

  // 0a. Supervisor events — these are the actual spawn/exit/crash signals
  //     emitted by the STDB supervisor. Most user-visible "what's happening"
  //     surface; emit one row per kind.
  for (const ev of args.supervisorEvents ?? []) {
    if (ev.id <= lastSupervisorEventId) continue;
    lastSupervisorEventId = Math.max(lastSupervisorEventId, ev.id);
    const evTime = ev.ts ? new Date().toLocaleTimeString().slice(0, 8) : time;
    let icon = "•", color = "text-blue-400", text = ev.kind;
    if (ev.kind === "spawn_request") {
      icon = "↑"; color = "text-green-400";
      text = `spawn ${ev.poolId}`;
    } else if (ev.kind === "process_exited") {
      icon = "↓"; color = "text-orange-400";
      const reason = (() => { try { return JSON.parse(ev.payload).reason ?? ""; } catch { return ""; } })();
      text = reason ? `exit ${ev.poolId} (${reason})` : `exit ${ev.poolId}`;
    } else if (ev.kind === "crash_loop") {
      icon = "✗"; color = "text-red-400";
      text = `crash-loop: ${ev.poolId}`;
    } else if (ev.kind === "tick") {
      // Skip noisy tick events — they fire every 10s.
      continue;
    }
    pushEvent({ ts: now, time: evTime, icon, color, source: "super", text });
  }

  // 0b. Pool state transitions (alive count / paused / in_crash_loop)
  for (const p of args.pools ?? []) {
    const prev = prevPoolState.get(p.id);
    const cur = {
      alive: p.aliveCount, desired: p.desiredCount,
      paused: p.paused, crash: p.inCrashLoop,
    };
    if (prev) {
      if (prev.alive !== cur.alive) {
        const delta = cur.alive - prev.alive;
        pushEvent({
          ts: now, time,
          icon: delta > 0 ? "↑" : "↓",
          color: delta > 0 ? "text-green-400" : "text-orange-400",
          source: "pool",
          text: `${p.id} ${prev.alive}→${cur.alive}/${cur.desired}`,
        });
      }
      if (!prev.crash && cur.crash) {
        pushEvent({ ts: now, time, icon: "✗", color: "text-red-400",
          source: "pool", text: `${p.id} entered CRASH LOOP` });
      } else if (prev.crash && !cur.crash) {
        pushEvent({ ts: now, time, icon: "✓", color: "text-green-400",
          source: "pool", text: `${p.id} recovered` });
      }
      if (prev.paused !== cur.paused) {
        pushEvent({ ts: now, time, icon: cur.paused ? "⏸" : "▶",
          color: "text-yellow-400", source: "pool",
          text: `${p.id} ${cur.paused ? "paused" : "resumed"}` });
      }
    }
    prevPoolState.set(p.id, cur);
  }

  // 1. Task status transitions
  const currentTaskStatus = new Map<string, string>();
  const currentTaskMeta = new Map<string, string>();
  for (const s of args.swarms) {
    for (const t of (s.tasks || [])) {
      currentTaskStatus.set(t.id, t.status || "");
      currentTaskMeta.set(t.id, t.title);
    }
  }
  for (const [tid, status] of currentTaskStatus) {
    const prev = prevTaskStatus.get(tid);
    if (!prev) continue;
    if (prev !== status) {
      const title = currentTaskMeta.get(tid) || tid.slice(0, 8);
      const titleShort = title.length > 60 ? title.slice(0, 57) + "…" : title;
      let icon = "→", color = "text-cyan-400";
      if (status === "completed" || status === "done") { icon = "✓"; color = "text-green-400"; }
      else if (status === "failed") { icon = "✗"; color = "text-red-400"; }
      else if (status === "blocked") { icon = "⛔"; color = "text-yellow-400"; }
      else if (status === "in_progress") { icon = "▶"; color = "text-cyan-400"; }
      pushEvent({
        ts: now, time, icon, color,
        source: "task",
        text: `${prev}→${status}: ${titleShort}`,
      });
    }
  }
  prevTaskStatus = currentTaskStatus;

  // 2. New swarms
  const currentSwarmIds = new Set(args.swarms.map((s) => s.id));
  if (prevSwarmIds.size > 0) {
    for (const sid of currentSwarmIds) {
      if (!prevSwarmIds.has(sid)) {
        const sw = args.swarms.find((s) => s.id === sid);
        pushEvent({
          ts: now, time, icon: "+", color: "text-purple-400",
          source: "swarm",
          text: `new: ${sw?.name || sid.slice(0, 12)}`,
        });
      }
    }
  }
  prevSwarmIds = currentSwarmIds;

  // 3. Decisions delta
  if (args.decisions) {
    const total = args.decisions.total;
    if (prevDecisionTotal >= 0 && total !== prevDecisionTotal) {
      const delta = total - prevDecisionTotal;
      pushEvent({
        ts: now, time,
        icon: delta > 0 ? "△" : "▽",
        color: delta > 0 ? "text-yellow-400" : "text-green-400",
        source: "decisions",
        text: `${delta > 0 ? "+" : ""}${delta} (now ${total})`,
      });
    }
    prevDecisionTotal = total;
  }

  // 4. Agent online count change
  const onlineCount = args.agents.filter(
    (a) => a.status === "online" || a.status === "active",
  ).length;
  if (prevAgentOnlineCount >= 0 && onlineCount !== prevAgentOnlineCount) {
    const delta = onlineCount - prevAgentOnlineCount;
    pushEvent({
      ts: now, time,
      icon: delta > 0 ? "●" : "○",
      color: delta > 0 ? "text-green-400" : "text-orange-400",
      source: "agents",
      text: `${delta > 0 ? "+" : ""}${delta} online (now ${onlineCount})`,
    });
  }
  prevAgentOnlineCount = onlineCount;
}

const EventFeed: Component<{
  swarms: () => Swarm[];
  decisions: () => DecisionsResponse | null;
  agents: () => { name?: string; status?: string }[];
  pools: () => PoolStatus[];
  supervisorEvents: () => SupervisorEvent[];
}> = (props) => {
  const [, forceRender] = createSignal(0);

  // Recompute events whenever any of our inputs change.
  createEffect(() => {
    deriveEvents({
      swarms: props.swarms(),
      decisions: props.decisions(),
      agents: props.agents(),
      pools: props.pools(),
      supervisorEvents: props.supervisorEvents(),
    });
    forceRender((n) => n + 1);
  });

  const reversed = () => [...eventBuffer].slice(-15).reverse();

  return (
    <footer class="border-t border-gray-800 bg-gray-950 px-4 py-1.5 flex items-center gap-3 text-[11px] overflow-x-auto whitespace-nowrap shrink-0">
      <span class="text-[10px] font-bold uppercase tracking-wider text-gray-400 shrink-0">Activity</span>
      <Show
        when={reversed().length > 0}
        fallback={<span class="italic text-gray-500">watching for state changes…</span>}
      >
        <For each={reversed()}>
          {(e) => (
            <span class="shrink-0">
              <span class="text-gray-500">{e.time}</span>
              <span class={`ml-1 ${e.color}`}>{e.icon}</span>
              <span class="ml-1 text-gray-400">{e.source}</span>
              <span class="ml-1 text-gray-200">{e.text}</span>
              <span class="text-gray-700 mx-2">·</span>
            </span>
          )}
        </For>
      </Show>
    </footer>
  );
};

// ── Main page ────────────────────────────────────────────────────────────────

const PROJECT_FILTER_KEY = "hex-brain-project-filter-v1";

const Brain: Component = () => {
  const [swarmsAll, setSwarmsAll] = createSignal<Swarm[]>([]);
  const [decisions, setDecisions] = createSignal<DecisionsResponse | null>(null);
  const [improver, setImprover] = createSignal<ImproverStatus | null>(null);
  const [agents, setAgents] = createSignal<{ name?: string; capabilities?: { role?: string } }[]>([]);
  const [projects, setProjects] = createSignal<ProjectInfo[]>([]);
  const [pools, setPools] = createSignal<PoolStatus[]>([]);
  const [supervisorEvents, setSupervisorEvents] = createSignal<SupervisorEvent[]>([]);
  // Keyed lookup so TeamRail can read each persona's pool by role name.
  const poolByRole = createMemo(() => {
    const m = new Map<string, PoolStatus>();
    for (const p of pools()) m.set(p.role, p);
    return m;
  });
  const scalePool = async (role: string, count: number) => {
    const existing = pools().find((p) => p.role === role);
    if (!existing) return;
    try {
      await restClient.post("/api/pools", {
        id: existing.id,
        role: existing.role,
        desired_count: count,
        restart_strategy: existing.restartStrategy,
        max_restarts: 5,
        max_restart_window_secs: 60,
        paused: false,
        owner_agent_id: "operator",
      });
      // Optimistic local update; refresh() will reconcile.
      setPools((all) => all.map((p) =>
        p.id === existing.id ? { ...p, desiredCount: count, paused: false, inCrashLoop: false } : p,
      ));
    } catch { /* surface in toast later */ }
  };
  // Lifted chat input — DecisionsPanel and KanbanLanes write to it on card
  // click (prefill an @-mention + question), ChatPanel reads and sends.
  const [chatInput, setChatInput] = createSignal("");
  // Auto-send token: when bumped, ChatPanel's effect detects it and dispatches
  // the current input. Used for click-to-ask flow from cards (one-click answer).
  const [autoSendBump, setAutoSendBump] = createSignal(0);
  // Helper for cards to dispatch to chat. Default is auto-send (one-click).
  // Operator can still edit and re-send by typing; the card flow is for
  // common questions where editing is unnecessary.
  const sendToChat = (text: string, role?: string, opts?: { autoSend?: boolean }) => {
    setChatInput(text);
    if (role) setSelectedAgent(role);
    if (opts?.autoSend !== false) {
      setAutoSendBump((n) => n + 1);
    } else {
      queueMicrotask(() => {
        const ta = document.querySelector("aside textarea") as HTMLTextAreaElement | null;
        if (ta) {
          ta.focus();
          ta.setSelectionRange(text.length, text.length);
        }
      });
    }
  };
  // Project filter — "" means "all projects". Persists in localStorage.
  const [projectFilter, setProjectFilter] = createSignal<string>(
    localStorage.getItem(PROJECT_FILTER_KEY) ?? "",
  );
  createEffect(() => {
    try { localStorage.setItem(PROJECT_FILTER_KEY, projectFilter()); } catch { /* quota */ }
  });
  // Lookup helpers
  const projectName = (id: string): string => {
    if (!id) return "global";
    const p = projects().find((p) => p.id === id);
    return p?.name ?? id.slice(0, 8);
  };
  // Filtered swarms — when projectFilter is "" show everything; "__global__"
  // shows only swarms with no project; else only swarms whose projectId matches.
  const swarms = createMemo(() => {
    const f = projectFilter();
    if (!f) return swarmsAll();
    if (f === "__global__") {
      return swarmsAll().filter((s) => !(s.projectId || s.project_id));
    }
    return swarmsAll().filter((s) => (s.projectId || s.project_id || "") === f);
  });
  // Selected agent — Brain owns this so TeamRail (left) and ChatPanel (right)
  // share state. Click a persona in the rail → chat opens with that agent
  // pre-selected. @-mention in the chat or click a different persona → updates.
  const [selectedAgent, setSelectedAgent] = createSignal<string | null>(null);

  const onlineNames = createMemo(() => {
    const set = new Set<string>();
    for (const a of agents()) {
      const role = a.capabilities?.role;
      if (role) set.add(role);
      if (a.name) {
        // names are like "pm-agent-bazzite.lan" — extract the role prefix
        for (const p of PERSONAS) {
          if (a.name.startsWith(p.name)) { set.add(p.name); break; }
        }
      }
    }
    return set;
  });

  const refresh = async () => {
    try {
      const s = await restClient.get<Swarm[]>("/api/swarms/active");
      setSwarmsAll(Array.isArray(s) ? s : []);
    } catch { /* nexus may be down */ }
    try {
      const p = await restClient.get<{ projects: ProjectInfo[] }>("/api/projects");
      setProjects(p?.projects ?? []);
    } catch { /* projects endpoint may not exist */ }
    try {
      const pp = await restClient.get<{ pools: PoolStatus[] }>("/api/pools");
      setPools(pp?.pools ?? []);
    } catch { /* pools table may not exist on first STDB sync */ }
    try {
      const se = await restClient.get<{ events: SupervisorEvent[] }>("/api/supervisor/events?limit=40");
      setSupervisorEvents(se?.events ?? []);
    } catch { /* supervisor_event table may not exist yet */ }
    try {
      // Project-scoped decisions when a filter is active. The aggregator
      // walks <projectRootPath>/docs/workplans + docs/adrs instead of cwd.
      const f = projectFilter();
      const url = f && f !== "__global__"
        ? `/api/decisions?project=${encodeURIComponent(f)}`
        : "/api/decisions";
      const d = await restClient.get<DecisionsResponse>(url);
      setDecisions(d);
    } catch { /* ignore */ }
    try {
      const i = await restClient.get<{ agents: any[] }>("/api/hex-agents");
      setAgents(i.agents || []);
    } catch { /* ignore */ }
    // improver status — best-effort, not all setups expose this REST surface.
    try {
      const im = await restClient.get<ImproverStatus>("/api/sched/improver/status");
      setImprover(im);
    } catch { /* improver not exposed via REST in all builds */ }
  };

  let pollHandle: number | undefined;
  onMount(() => {
    refresh();
    pollHandle = window.setInterval(refresh, 15000);
  });
  onCleanup(() => { if (pollHandle !== undefined) window.clearInterval(pollHandle); });
  // Re-fetch when the project filter changes (decisions endpoint takes the
  // project as a query param). createEffect tracks projectFilter() automatically.
  createEffect(() => {
    projectFilter();
    refresh();
  });

  return (
    <div class="flex flex-col h-screen bg-gray-950 text-gray-100">
      {/* Top bar */}
      <header class="px-4 py-2 border-b border-gray-800 flex items-center gap-3 shrink-0">
        <h1 class="text-sm font-bold tracking-wide text-gray-200">HEX BRAIN</h1>
        <span class="text-[11px] text-gray-300">
          {PERSONAS.length} personas · {swarms().length} active swarms · {decisions()?.total ?? 0} decisions
        </span>
        {/* Project filter — scopes Kanban + Swarms to one project. Decisions
            currently come from cwd's docs/workplans so they're already
            scoped server-side. */}
        <label class="ml-auto flex items-center gap-1.5 text-[10px] text-gray-300">
          <span class="uppercase tracking-wider">Project:</span>
          <select
            value={projectFilter()}
            onChange={(e) => setProjectFilter(e.currentTarget.value)}
            class="bg-gray-900 border border-gray-800 rounded px-1.5 py-0.5 text-[11px] text-gray-200 focus:outline-none focus:border-cyan-700"
          >
            <option value="">all projects ({swarmsAll().length})</option>
            <For each={projects()}>
              {(p) => {
                const count = swarmsAll().filter(
                  (s) => (s.projectId || s.project_id || "") === p.id,
                ).length;
                return <option value={p.id}>{p.name} ({count})</option>;
              }}
            </For>
            {(() => {
              const globals = swarmsAll().filter(
                (s) => !(s.projectId || s.project_id),
              ).length;
              return globals > 0 ? <option value="__global__">global ({globals})</option> : null;
            })()}
          </select>
        </label>
        <span class="text-[10px] text-gray-400">refresh: 15s</span>
      </header>

      {/* Three-pane main */}
      <main class="flex flex-1 overflow-hidden">
        <TeamRail
          onlineNames={onlineNames}
          onSelect={setSelectedAgent}
          selected={selectedAgent}
          poolByRole={poolByRole}
          onScale={scalePool}
        />

        <div class="flex-1 overflow-y-auto px-4 py-4">
          {/* Decisions placed at the top of the center stack — it's the
              priority lane (human-in-loop bottlenecks). Kanban next for
              flow visibility. Swarms + Health below as supplementary. */}
          <DecisionsPanel data={decisions} onSendToChat={sendToChat} />
          <KanbanLanes swarms={swarms} onSendToChat={sendToChat} onTaskMoved={refresh} />
          <div class="grid grid-cols-2 gap-4">
            <SwarmsPanel swarms={swarms} projectName={projectName} />
            <HealthPanel improver={improver} swarmCount={() => swarms().length} />
          </div>
        </div>

        <ChatPanel
          selectedAgent={selectedAgent}
          onAgentChange={setSelectedAgent}
          projectId={projectFilter}
          projectName={() => projectName(projectFilter())}
          externalInput={chatInput}
          setExternalInput={setChatInput}
          autoSendBump={autoSendBump}
        />
      </main>

      <EventFeed swarms={swarms} decisions={decisions} agents={agents} pools={pools} supervisorEvents={supervisorEvents} />
    </div>
  );
};

export default Brain;
