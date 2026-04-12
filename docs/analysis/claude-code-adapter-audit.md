# P4.1 — ClaudeCodeInferenceAdapter Audit

**Workplan**: `wp-hex-standalone-dispatch`
**ADR**: `ADR-2604112000` (Hex Self-Sufficient Dispatch / Standalone Mode)
**Author**: hex-coder (P4.1 bundled with P4.2/P4.3)
**Date**: 2026-04-11
**Builds on**: `docs/analysis/inference-trait-audit.md` §3 — do not re-litigate.

---

## 1. Existing code summary

`hex-agent/src/adapters/secondary/claude_code_inference.rs` today exposes:

- `pub fn is_claude_code_session() -> bool` — sniffs `CLAUDECODE=1` or
  `CLAUDE_CODE_ENTRYPOINT`.
- `pub fn which_claude() -> Option<PathBuf>` — walks `$PATH` looking for a
  `claude` binary.
- `pub struct ClaudeCodeInferenceAdapter { project_dir: String }` with
  `pub async fn run_task(&self, prompt: &str) -> Result<String, String>` —
  one-method shim that spawns `claude -p <prompt> --output-format text
  --dangerously-skip-permissions` in `project_dir` and returns stdout on
  exit 0, stderr-wrapped error on non-zero.

The struct does **not** implement `IInferencePort` — it is not a port
adapter at all, just a subprocess helper. No `complete`, no `stream`, no
`health`, no `capabilities`, no `InferenceRequest` / `InferenceResponse`
types anywhere in the file.

**Callers** (grep across the workspace):

- `hex-agent/src/main.rs:24` — `use adapters::secondary::claude_code_inference::{is_claude_code_session, which_claude, ClaudeCodeInferenceAdapter};`
- `hex-agent/src/main.rs:188-189` — `use_bypass = !args.no_claude_code_bypass && is_claude_code_session() && which_claude().is_some();`
- `hex-agent/src/main.rs:710` — `let adapter = ClaudeCodeInferenceAdapter::new(&args.project_dir); match adapter.run_task(prompt).await { ... }`

There are **zero** callers in `hex-nexus` or `hex-cli`. The module is a
hex-agent-private helper, not a fleet-level adapter.

## 2. Classification — every `CLAUDE_SESSION_ID` / session / spawn reference

| Site | What it does | Classification |
|---|---|---|
| `hex-agent/.../claude_code_inference.rs:9` `CLAUDECODE` env read | sniffs whether we are inside a Claude Code session | **COMPOSITION** — stays in hex-agent main.rs as a branching flag; hex-nexus composition has its own `orchestration::is_claude_code_session` (`hex-nexus/src/orchestration/mod.rs:62`) for the same purpose. P2's `compose_auto` already consults it. |
| `hex-agent/.../claude_code_inference.rs:26-38` `tokio::process::Command` spawn of `claude -p ... --dangerously-skip-permissions` | actually runs Claude Code as a subprocess | **ADAPTER** — the spawn itself is exactly what a backend adapter does. Moves into `hex-nexus/src/adapters/inference/claude_code.rs` as the new `ClaudeCodeInferenceAdapter`. |
| `hex-agent/.../claude_code_inference.rs:50-56` `which_claude` $PATH walk | binary discovery | **ADAPTER** — the new adapter's `health()` handles this via the injected `ProcessSpawner` (missing binary → spawn error → `HealthStatus::Unreachable`). |
| `hex-nexus/src/orchestration/mod.rs:62` `is_claude_code_session` | hex-nexus's own CLAUDECODE probe | **COMPOSITION** — already lives in the composition layer (P2). Out of P4 scope. |
| `hex-nexus/src/routes/orchestration.rs:369,405` `is_claude_code_session` branching | route-time fast-path selection | **COMPOSITION** — stays. P4 must NOT touch. |
| `hex-nexus/src/orchestration/workplan_executor.rs:789` `is_claude_code_session` → path_b | executor picks bypass | **COMPOSITION** — stays. |
| `CLAUDE_SESSION_ID` reads (any file) | — | **NOT FOUND** in the claude_code_inference module. The ADR explicitly warns adapters against reading this; the old module already doesn't, and the new one must not. (The identifier lives in session hooks and composition code only.) |
| `~/.hex/sessions/agent-*.json` reads | — | **NOT FOUND** in claude_code_inference. Same rule: composition only. |

## 3. Migration decision — **option (a): keep hex-agent's version as-is**

hex-agent's `main.rs` is a live consumer — deleting the module would break
`hex-agent --prompt "..."` when CLAUDECODE=1. The new nexus adapter is a
**parallel** backend that lives alongside it in a different crate and
solves a different problem (implementing `IInferencePort` for the
composition root per ADR-2604112000), so there is no collision.

Three sentences of justification:

1. **Scope discipline.** The P4 task description says "rework as a backend,
   not a shell" — the new file in `hex-nexus/src/adapters/inference/` is
   the backend; the existing hex-agent shim is the shell, and the ADR
   says shells and backends are different concerns.
2. **No behavioural duplication.** hex-agent's `run_task` is a fire-and-
   forget one-shot that returns a `Result<String, String>` — it has no
   request type, no streaming, no `InferenceCapabilities`. The new adapter
   owes none of that legacy surface to hex-agent's caller.
3. **Blast radius.** Touching hex-agent means touching `main.rs`, which
   means retesting the entire hex-agent path. Leaving hex-agent alone is
   one diff smaller and keeps P4 focused on the nexus port impl.

If a future phase (P7+?) wants to collapse the two, the hex-agent shim can
become a ~30-line wrapper that constructs `hex_nexus::adapters::inference::
ClaudeCodeInferenceAdapter` and calls `.complete()`. That is a **follow-up**,
not P4 work.

## 4. Known bug — `claude -p` exits 1 without `--dangerously-skip-permissions`

Per `feedback_claude_bypass_permissions` memory:

> `claude -p` exits 1 without `--dangerously-skip-permissions` when spawned
> non-interactively; fix in `ClaudeCodeInferenceAdapter`.

**Current state of hex-agent's version**: it already passes the flag (line
31), so hex-agent itself is not affected. The bug is latent in the sense
that **any new caller that forgets to pass the flag** would hit it.

**Fix in the new nexus adapter**: the flag is hardcoded unconditionally in
the `args_for_prompt` helper — no constructor knob, no env-var escape
hatch, no "is this a TTY?" branch. Every `complete` and `stream` call
passes it. Regression test case #2 (`always_passes_dangerously_skip_permissions`)
asserts it.

## 5. Test strategy for P4.3

The new adapter must be testable without a real `claude` binary and with
zero subprocess spawns. The design:

```rust
#[async_trait]
pub trait ProcessSpawner: Send + Sync {
    async fn spawn(&self, program: &str, args: &[String])
        -> Result<SpawnedProcess, InferenceError>;
}

pub struct SpawnedProcess {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
}
```

- **Production** uses `TokioProcessSpawner`, which calls
  `tokio::process::Command::new(program).args(args).output().await` and
  maps the `std::io::Error` kinds onto `InferenceError` variants
  (NotFound → `ProviderUnavailable { reason: "binary not found" }`,
  other → `Network`).
- **Tests** construct `MockProcessSpawner::with_responses(VecDeque<...>)`
  (a public `testing` submodule so the integration-test crate can reach
  it), pushing canned `SpawnedProcess` records or errors. Each test
  drains its responses in order.
- **Streaming** goes through a second method `spawn_streaming(program,
  args) -> Result<(Vec<String> /* lines */, i32 /* exit */), InferenceError>`
  on the same trait. This keeps the trait surface minimal and avoids
  inventing a `Box<dyn AsyncRead>` return type that the mock would have
  to fake up. The real `TokioProcessSpawner` impl reads stdout to
  completion with `BufReader::lines()` and returns the collected lines;
  the tests just return a `Vec<String>` directly.

Five cases per the workplan: happy path, flag presence, binary missing,
stream order, non-zero exit.

Non-goal: the adapter does **not** stream line-by-line in real time over a
channel the way Ollama does. Claude's `-p --output-format text` flushes
the complete response on process exit, so there is no mid-stream consumer
to service — every token arrives when the process ends. The `stream()`
impl emits each stdout line as a `TextDelta` and then a `MessageStop`, all
synthesised from the post-exit stdout. If a future Claude flag exposes
real-time NDJSON, swap `TokioProcessSpawner::spawn_streaming` to a
child-piped reader — the trait shape accommodates it.
