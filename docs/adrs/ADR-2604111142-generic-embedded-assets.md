# ADR-2604111142: Generic-Only Embedded Assets in hex-cli/assets/

**Status:** Proposed
**Date:** 2026-04-11
**Drivers:** `hex assets sync` audit on 2026-04-11 revealed that `hex-cli/assets/` — the template set embedded into the binary via `rust-embed` and shipped to every target project via `hex init` / `hex assets sync` / `hex scaffold` — contains skills, agents, hooks, context templates, and CI configs that are **hex-maintainer runbooks**, not project-agnostic templates. Specifically, `hex-cli/assets/skills/hex-publish-module/SKILL.md` shipped literal `cd /Volumes/ExtendedStorage/PARA/01-Projects/hex-intf` and `cargo build -p hex-nexus -p hex-cli --release` instructions to any target project that ran `hex init`. This is an AIOS positioning failure: hex claims to be the OS layer for *other* projects, but its templates assume every consumer is hex-intf itself.
**Supersedes:** None (complements ADR-2603221522 `hex assets` embedded-asset CLI, ADR-049 MCP/settings template)

<!-- ID format: YYMMDDHHMM — 2604111142 = 2026-04-11 11:42 local, assigned by `hex adr schema` -->

## Context

`hex-cli/assets/` is baked into the `hex` binary at compile time via
`rust-embed`. Two consumption paths exist:

1. **`hex assets sync`** (`hex-cli/src/commands/assets_cmd.rs`) — walks three
   prefixes (`agents/`, `skills/`, `helpers/`) and writes each embedded file
   into a target project's `.claude/` directory. Modified local files are
   preserved unless `--force` is passed.
2. **`hex init` / `hex scaffold`** — reads from `hex-cli/assets/scaffold/`,
   a parallel `.claude/` subtree that seeds a freshly-initialised target
   project. Today this subtree duplicates much of what `skills/` contains,
   with its own drift.

During an audit on 2026-04-11 we grepped both trees for hex-intf-specific
markers (`/Volumes/`, `hex-intf`, `hex-nexus`, `hex-cli`, `hex-core`,
`spacetime-modules/`, `cargo build -p hex-*`). The results:

### Fundamentally hex-maintainer-only (should never have been embedded)

- `hex-cli/assets/skills/hex-publish-module/SKILL.md` — 19 hex-intf markers,
  including a hardcoded `/Volumes/ExtendedStorage/PARA/01-Projects/hex-intf`
  path and the literal list of the seven hex spacetime modules. Deleted
  2026-04-11 after confirming the file is preserved under hex-intf's live
  `.claude/skills/`.
- `hex-cli/assets/skills/hex-spacetime/SKILL.md` — 27 markers. Describes
  "use hex-nexus as the filesystem bridge for your WASM modules" as if
  every target project has a hex-nexus daemon. Deleted.
- `hex-cli/assets/skills/hex-dev-rebuild.md` — entire content is
  "rebuild the hex-nexus binary after Rust code changes". Frontmatter
  description: `Rebuild and deploy hex-nexus binary after Rust code or
  asset changes`. Deleted.
- `hex-cli/assets/scaffold/.claude/skills/hex-{publish-module,spacetime,dev-rebuild}/`
  — parallel duplicates of the same three skills in the scaffold tree.
  Deleted in the same pass.

### Pre-existing leaks still embedded (not fixed yet — scope of this ADR's workplan)

- `hex-cli/assets/skills/hex-adr-create.md` — references `hex-nexus/`
  paths in example commands
- `hex-cli/assets/skills/hex-analyze-arch/SKILL.md` — references
  `hex-nexus`, `hex-cli` crates in examples
- `hex-cli/assets/skills/hex-feature-dev/SKILL.md` — references the
  hex-intf worktree layout and Rust workspace specifics
- `hex-cli/assets/agents/hex/hex/adversarial-reviewer.yml` — references
  `cargo build -p hex-nexus`, `hex-core`, `hex-cli` by name in its
  review checklist
- `hex-cli/assets/hooks/hex/hex-adr-lifecycle.yml` — references
  `hex-nexus/src/` paths
- `hex-cli/assets/hooks/hex/hex-no-rest-state-mutation.yml` — references
  `hex-nexus` routes
- `hex-cli/assets/context-templates/tools/{read,grep,edit}.md` — references
  `hex-nexus`, `hex-cli` in examples
- `hex-cli/assets/context-templates/services/hexflo-global.md` — hardcoded
  hex-intf paths
- `hex-cli/assets/context-templates/services/session-memory.md` —
  `cargo build -p hex-nexus` specific commands
- `hex-cli/assets/ci/hex-ci.yml` — references hex-intf CI layout

### What's broken without cleanup

- **Every target project gets hex-intf-branded templates.** A user running
  `hex init` in a Python or TypeScript project receives skills that
  instruct them to `cd spacetime-modules/` (which doesn't exist) and
  `cargo build -p hex-nexus` (wrong language, wrong crate).
- **Nothing stops the drift from reappearing.** There is no CI gate, no
  `hex doctor` check, and no test that enforces "embedded assets contain
  no hex-intf internal references". A future edit can re-introduce the
  pollution and it will ship.
- **The scaffold subtree is silently out of sync with the primary
  `skills/` tree.** Two templates for the same skill can drift
  independently, and no one notices until a sync check is run.
- **The failure mode is silent.** Target projects just end up with
  broken skills; the user has no way to know their `.claude/skills/`
  was contaminated by the hex binary itself.

## Decision

Embedded assets under `hex-cli/assets/` MUST be **project-agnostic**. The
rule is enforced in three ways:

1. **Separation mechanism (structural, already in place).** A file is
   "shipped to target projects" iff it exists under `hex-cli/assets/`.
   A file that lives only in hex-intf's live `.claude/` is invisible to
   `hex assets sync`, `hex init`, and `hex scaffold` — because the sync
   tool walks `Assets::iter()` (the embedded set) as the source of
   truth, classifying target files only against the embedded set. Files
   that exist only locally in `.claude/` are neither written,
   overwritten, nor reported as drift. We do not need a new flag, a
   new directory convention, or a new manifest — absence from the
   embedded set is the marker.

2. **Content rule (normative).** Files under `hex-cli/assets/` must not
   contain any of the following hex-intf-specific markers:
   - Absolute paths starting with `/Volumes/`
   - The literal string `hex-intf`
   - Hex internal crate names as build targets: `hex-nexus`, `hex-cli`,
     `hex-core`, `hex-agent`, `hex-parser`, `hex-desktop` (appearing
     in `cargo build -p <crate>` or as filesystem path components
     like `hex-nexus/src/`)
   - `spacetime-modules/` paths (hex's internal WASM module directory)
   - References to hex's own 7-module spacetime topology by name
     (`hexflo-coordination`, `agent-registry`, `inference-gateway`,
     `secret-grant`, `rl-engine`, `chat-relay`, `neural-lab`)

   Exceptions: references to the `hex` CLI binary itself (`hex analyze`,
   `hex plan execute`, `hex init`) are allowed and expected — those
   are the user-facing interface every target project inherits. What's
   forbidden is references to hex's *implementation*, not its
   *interface*.

3. **Guardrail (automated).** `hex doctor` grows a new check,
   `embedded-assets-generic`, that walks `Assets::iter()` and greps
   every embedded file for the marker set above. Any hit is a doctor
   failure with a clear message ("`hex-cli/assets/skills/foo.md:42`
   contains hex-intf-specific marker `hex-nexus/src/` — move the
   file to `.claude/skills/` or genericize the reference"). `hex ci`
   runs `hex doctor` in strict mode and fails the build on any such
   hit. This is the guardrail that prevents the drift from recurring.

## Consequences

**Immediate (completed 2026-04-11):**
- `hex-publish-module`, `hex-spacetime`, `hex-dev-rebuild` deleted
  from `hex-cli/assets/skills/` and `hex-cli/assets/scaffold/.claude/skills/`.
- Live copies preserved in hex-intf's own `.claude/skills/`.
- `cargo check -p hex-cli` verified green post-delete.

**Deferred to workplan `wp-embedded-assets-genericization`:**
- Audit and genericize the 10+ files identified in the pre-existing-leaks
  list above. Each file must either (a) have its hex-intf markers replaced
  with language-agnostic wording, or (b) be deleted from `hex-cli/assets/`
  and preserved in `.claude/` if it is fundamentally hex-intf-only.
- Reconcile `hex-cli/assets/scaffold/.claude/` against `hex-cli/assets/skills/`
  so the two trees converge on the same generic content (or document
  why they must diverge — e.g. scaffold tree may contain init-specific
  seed files not in the sync set).
- Implement the `hex doctor embedded-assets-generic` check and wire it
  into `hex ci`.

**Target-project impact:** Any target project that ran `hex init` or
`hex assets sync` before this cleanup has the polluted skills in its
local `.claude/`. They are NOT automatically removed — `hex assets sync`
never deletes local files that aren't in the embedded set. Affected
users must manually delete `.claude/skills/hex-publish-module/`,
`.claude/skills/hex-spacetime/`, and `.claude/skills/hex-dev-rebuild.md`
from their projects, or run a future `hex assets prune` command (not
yet designed, potential future ADR).

**hex-intf itself is unaffected.** The three deleted skills still live
in hex-intf's own `.claude/skills/` and work exactly as before for
hex maintenance tasks (publishing WASM modules, creating spacetime
modules, rebuilding hex-nexus). This ADR explicitly preserves the
"hex-intf can use hex-intf-specific skills" axis while closing the
"hex-intf-specific skills leak to every other project" axis.

## Alternatives Considered

- **(A) Tag files with a frontmatter `scope: hex-intf-only` field and
  have `rust-embed` filter them at compile time.** Rejected: requires
  parsing YAML/Markdown at build time, adds a new concept ("scope")
  that's easy to forget, and the structural mechanism (absence from
  the embed tree) is already sufficient. Less machinery is better.

- **(B) Move hex-intf-specific skills to a separate `hex-cli/assets-dev/`
  tree and have `rust-embed` only pick up `hex-cli/assets/` in release
  builds.** Rejected: introduces a conditional compile step, requires
  a dev-only embed tree the release binary never sees, and hex-intf's
  own `.claude/` already serves this role without any build system
  changes.

- **(C) Keep everything in `hex-cli/assets/` and just genericize the
  hex-intf-specific skills in place (replace `hex-nexus` with
  `<your-host-bridge>`, etc.).** Rejected for the three deleted
  skills: `hex-publish-module` is literally a runbook for publishing
  hex's own spacetime modules — there is no "generic" equivalent
  because no other project has hex's specific 7-module topology.
  The skill has no project-agnostic form. Kept as the strategy for
  the remaining 10 files in the workplan where the core content IS
  generic and only needs to strip hex-intf examples.

- **(D) Do nothing — let target projects ignore skills they don't
  need.** Rejected because skills aren't inert — Claude Code may
  invoke them based on the frontmatter `description` field, which
  says things like `"spacetimedb", "wasm module", "new reducer"`.
  A target project with no SpacetimeDB at all will see its agent
  get routed into a hex-intf-specific runbook and follow literal
  `/Volumes/ExtendedStorage/…` instructions. This is worse than
  "doesn't help" — it's actively misleading.

## Notes

- The `hex assets sync --force` direction (embedded → live overwrite)
  is unchanged by this ADR. It is still the correct behavior for
  target projects that want to re-sync to the canonical embedded
  templates.
- The reverse direction (live → embedded), which does not have a
  dedicated CLI command, is governed by the content rule above: only
  mirror a live file into the embed set after confirming it passes the
  marker check. This is captured in the `feedback_assets_sync_direction`
  memory and will be reinforced by the `hex doctor` guardrail in
  P7 of the workplan.
- Related ADRs: ADR-2603221522 (hex assets embedded-asset CLI introduction),
  ADR-049 (MCP config and settings template embedding). This ADR does
  not contradict either — it adds a content rule on top of the existing
  embedding mechanism.
