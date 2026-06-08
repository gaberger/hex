# ADR-2606071702: `hex dev deploy` verb + installed-binary freshness gate — close the validate≠deploy gap

**Status:** Completed
**Date:** 2026-06-07
**Epoch:** single-agent
**Drivers:** Deploying the ADR-2606071651 (③) fix this session exposed a gap: `hex dev
validate` reported **4/4 passed** (build → test → analyze → 39 specs), yet the installed
`~/.local/bin/hex` stayed at the *pre-change* build (size 13514376, 13:58) — the fix was
not live. Making it live required a hand-run `cargo build -p hex-cli --release` + `cp` to
`~/.local/bin`. There is **no `hex` verb that builds-and-installs the local binaries**, so a
green `validate` is routinely mistaken for "deployed."
**Supersedes:**
**Superseded-By:**

## Context

`hex dev validate` runs `build → test → analyze → validate(specs)`. Its **test** stage
compiles and runs in the **debug** profile (where a change is present the moment it is
written), so it can pass while the **release** artifact and the **installed** binary on
`$PATH` are stale. The binary the operator actually runs is `~/.local/bin/hex` — which
`validate` never rebuilds or refreshes. Result: *"validated" reads as "deployed" when it is
not.* This is the same **false-live** failure class that ADR-2606071651 (③) just fixed for
nexus binary *resolution* — one layer up:

- **③ (ADR-2606071651)** answers *"of the builds on disk, which one runs?"* → newest mtime,
  warn on shadow.
- **This ADR** answers *"does the build on disk / on `$PATH` reflect current source at
  all?"* → today: nothing checks, and nothing builds+installs it through a hex verb.

The existing surfaces don't close it:
- `hex dev validate` — validates correctness, does not deploy.
- `hex self-update` (ADR-2026-04-08-0929) — installs from **GitHub releases**, not the
  local working tree; useless for deploying an un-released local fix.
- `scripts/rebuild.sh` — build tooling, but copies to project-local `./bin/` (not the
  `~/.local/bin` that is on `$PATH`) and still references a **stale `hex-chat` crate** that
  is no longer part of the workspace; it will break or mis-deploy.
- `scripts/install.sh` — resolves the correct `BIN_DIR` (`~/.local/bin` on immutable
  distros) but is a one-time installer, not the iterative deploy path.

So the canonical "ship a local change to the running tools" operation is currently
ad-hoc `cargo build --release` + manual copy — exactly the kind of by-hand step that
HARD RULE 0 ("everything routes through hex") says to replace with a verb.

## Decision

Introduce a first-class deploy verb and a freshness gate so "it's live" is verifiable.

1. **`hex dev deploy`** — the canonical build-and-install verb. It release-builds the
   *current* workspace binaries (`hex-cli`, `hex-nexus`, `hex-agent` — enumerated from the
   workspace, never a hardcoded/stale list), installs them to the resolved `BIN_DIR`
   (reusing `install.sh`'s prefix logic: `~/.local/bin`, or `$PREFIX`), and restarts the
   managed services. Idempotent. Flags: `--no-restart` (install without bouncing the
   daemon), `--check` (report what *would* be deployed and the current drift, build
   nothing). This replaces `scripts/rebuild.sh` (which is then deprecated).

2. **Installed-binary freshness gate.** Embed a build hash in `hex-cli` at compile time via
   `build.rs` (mirroring the `--build-hash` mechanism hex-nexus already exposes and that ③'s
   `get_disk_build_hash` already reads). A freshness check compares the **installed**
   binary's `--build-hash` against the source HEAD (the latest commit touching that crate).
   Surface it:
   - in **`hex doctor`** (ADR-067, installation/pipeline verification) as a check, and
   - as a **non-fatal warning appended to `hex dev validate`**: e.g.
     *"validated ✓ — but installed `hex` is N commits behind source; run `hex dev deploy`."*

   `validate` stays a correctness gate (it does not auto-deploy — that would surprise CI and
   shared-host workflows), but it can no longer be *silently* mistaken for a deploy.

Together with ③ this makes the live state checkable end to end: ③ guarantees the freshest
on-disk build is the one that runs; this ADR guarantees the on-disk/installed build reflects
current source and lives on `$PATH`.

## Consequences

**Positive**
- One verb to ship a local change; no more by-hand `cargo build --release` + `cp`.
- A green `validate` can never again be mistaken for "deployed" — the drift is named.
- CI can assert freshness (`hex doctor` / `hex dev deploy --check`) as a release gate.
- Kills the stale `rebuild.sh` (`./bin` target + dead `hex-chat` reference) footgun.

**Negative / risks**
- `hex dev deploy` must resolve `BIN_DIR` correctly across distros — mitigated by reusing
  the already-shipped `install.sh` logic rather than re-deriving it.
- Embedding a build hash in `hex-cli` needs a small `build.rs` (git-hash at compile time,
  with a clean fallback when not in a git tree) — low risk, hex-nexus already does this.
- Service restart implies brief daemon downtime — gated behind the default; `--no-restart`
  for hot-swap-then-bounce-later workflows.
- A freshness check keyed on "commits touching the crate" can false-positive on no-op
  commits (formatting); acceptable — it warns, never blocks, and the fix is to deploy.

## Implementation

- `hex-cli` — add `DevAction::Deploy` (`hex dev deploy`) in the `dev` command module:
  enumerate workspace bins → `cargo build --release -p …` → install to resolved `BIN_DIR`
  → restart services unless `--no-restart`; implement `--check`.
- `hex-cli/build.rs` — embed `GIT_BUILD_HASH`; expose `hex --build-hash` (mirror hex-nexus).
- Freshness check — shared helper comparing installed `--build-hash` for `hex` and
  `hex-nexus` against `git rev-parse` of the last commit touching each crate; wired into
  `hex doctor` and appended to the `hex dev validate` summary as a warning line.
- Deprecate `scripts/rebuild.sh` with a pointer to `hex dev deploy`.
- Tests: `--check` reports drift on a deliberately-stale install; `build.rs` hash present and
  surfaced by `--build-hash`; freshness helper classifies behind/current correctly.

Tracking workplan: create via `hex plan draft` when scheduled.

## References

- ADR-2606071651 — ③, the sibling false-live fix at the binary-**resolution** layer
  (`find_nexus_binary` newest-mtime + shadow warning); this ADR is the build/install layer.
- ADR-2026-04-08-0929 — `hex self-update` (installs from GitHub releases; this verb is the
  local-working-tree complement).
- ADR-067 — `hex doctor` installation/pipeline verification (where the freshness check lands).
- Live evidence (this session): `hex dev validate` → 4/4 passed while `~/.local/bin/hex`
  remained the pre-③ build (13514376, 13:58); manual `cargo build --release` + `cp` required
  to make ③ live (installed 13532488, 17:01); ③'s own warning fired correctly on the
  subsequent restart.
