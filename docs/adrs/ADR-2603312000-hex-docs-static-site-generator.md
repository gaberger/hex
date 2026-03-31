# ADR-2603312000: hex docs — Static Site Generator for the hex Manual

**Status:** Proposed
**Date:** 2026-03-31
**Deciders:** hex core team

---

## Context

hex has grown to 32 CLI commands, 20 embedded skills, 14 agent definitions, 100+ ADRs, and a full development pipeline. There is no canonical reference manual — users must rely on `--help` output, scattered markdown files, and source code. This creates onboarding friction and makes the system hard to reason about holistically.

The docs must stay in sync with the binary automatically. Manual documentation quickly drifts. The source of truth for command structure is the clap `Command` tree; for skills, the embedded YAML/Markdown frontmatter; for ADRs, the `docs/adrs/` directory. A static site generator that reads from these sources directly eliminates drift by design.

---

## Decision

Add a `hex docs` subcommand to hex-cli that generates a self-contained static HTML site — the hex manual.

The generator:
1. **Introspects the live clap `Command` tree** to enumerate all commands, subcommands, flags, and descriptions — same source as `--help`, never out of sync
2. **Reads embedded assets** (skills, agent YAMLs, swarm YAMLs) from the rust-embed bundle already in the binary
3. **Reads ADRs** from `docs/adrs/` on the filesystem, renders markdown to HTML
4. **Outputs a self-contained static site** — a single directory with `index.html`, CSS, and no external dependencies (no CDN, no JavaScript frameworks)
5. **Optionally served** by hex-nexus at `/docs` alongside the dashboard

### Command interface

```
hex docs generate [--output <dir>]   # default: ./hex-docs/
hex docs serve                        # serve via hex-nexus (opens browser)
hex docs --version                    # show docs schema version
```

### Site structure

```
hex-docs/
  index.html              # landing page (overview + quick-start)
  commands/
    index.html            # all commands A-Z
    dev.html              # hex dev (all subcommands, flags, examples)
    git.html
    skill.html
    ...                   # one page per top-level command
  skills/
    index.html            # all skills with trigger + description
    hex-scaffold.html     # individual skill pages with full content
    ...
  agents/
    index.html            # all agent roles
    hex-coder.html        # YAML source + description
    ...
  adrs/
    index.html            # ADR index by status (Accepted / Proposed / Deprecated)
    ADR-001.html          # individual ADR pages
    ...
  concepts/
    hexagonal-architecture.html
    pipeline-phases.html
    swarm-topology.html
  static/
    style.css             # single self-contained stylesheet
    search.js             # client-side search index (JSON)
```

---

## Implementation

### Rust crates (all already in workspace or stdlib)

| Need | Crate |
|------|-------|
| Markdown → HTML | `pulldown-cmark` |
| HTML templating | `minijinja` (or hand-rolled string formatting) |
| CLI introspection | `clap::Command::get_subcommands()` (live tree walk) |
| Embedded assets | existing `rust-embed` bundle |
| File output | `std::fs` |

### Key design choices

**No JavaScript framework.** The site must render without JS. A small `search.js` for client-side filtering is the only JS allowed.

**Single CSS file.** Inline or single external `style.css` — no Tailwind build step, no PostCSS.

**Clap introspection over code generation.** Walking the live `clap::Command` tree at runtime means docs are always in sync — no separate doc comments to maintain.

**Markdown passthrough for ADRs.** ADR files are rendered as-is via `pulldown-cmark`. No reformatting.

**Version stamped.** Each generated site embeds the hex version and generation timestamp so users can tell if docs are stale.

---

## Consequences

**Positive:**
- Zero-drift documentation — generated from the same binary users run
- No external toolchain required to generate docs (no Node, no Ruby, no Python)
- Self-contained output can be hosted anywhere (GitHub Pages, S3, local)
- hex-nexus can serve it at `/docs` with no additional infrastructure

**Negative:**
- HTML generation in Rust requires care for escaping and template maintenance
- Client-side search is limited compared to Algolia/lunr — acceptable for a CLI tool manual

**Neutral:**
- ADR pages rendered from filesystem, not embedded — requires docs/ to be present at generation time (expected for development environments)

---

## Alternatives Considered

| Alternative | Rejected because |
|-------------|-----------------|
| mdBook | Requires manual content authoring; doesn't auto-generate from clap |
| Zola/Hugo | Requires separate toolchain install; same manual authoring problem |
| hex-nexus live docs endpoint | Only available when nexus is running; no offline/shareable output |
| rustdoc | Generated from `///` comments, not user-facing CLI docs |

---

## References

- ADR-2603221522: Embedded assets (rust-embed)
- ADR-2603221959: Enforcement rules
- All 32 `hex --help` command trees
- `hex-cli/assets/skills/` — 20 embedded skill definitions
- `hex-cli/assets/agents/hex/hex/` — 14 agent YAML definitions
