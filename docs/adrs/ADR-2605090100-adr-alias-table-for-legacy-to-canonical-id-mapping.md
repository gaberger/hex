# ADR-2605090100 — ADR Alias Table for Legacy-to-Canonical ID Mapping

Status: **Proposed**
Date: 2026-05-10

## Context

ADR-2603221500 (Timestamp-Based ADR Numbering) introduced YYMMDDHHMM IDs to eliminate sequential numbering races. The transition created **two valid ID formats in the wild**:
- **Legacy**: `ADR-2603221500` (10-digit timestamp)
- **Canonical (new)**: `ADR-2026-03-22-timestamp-adr-numbering` (YYYY-MMDD-slug filename convention)

Today's state:
- **Filenames** on disk use the new canonical form with slug (e.g. `ADR-2605082500-typed-tool-library-and-sop-execution.md`)
- **Tools** extract just the timestamp digits via `extract_adr_id()` in `hex-cli/src/commands/adr/mod.rs:449` and workplan_auto_emitter coverage scan (line 93)
- **References** in workplan JSON `adr` fields and operator notes use legacy `ADR-2603221500` format because that's what tooling emits
- **Ambiguity**: given a legacy ID like `ADR-2603221500`, humans cannot determine the slug without filesystem lookup

**Trigger**: original ask msg 56ea88ae hit OpenRouter HTTP 403 content-filter due to multiple consecutive 10-digit runs being misclassified as sensitive data. The redactor mangled them into invalid tool args, breaking the SOP REASON phase.

**Alternatives rejected**:
- **(A) Rename all files back to timestamp-only** → Rejected: breaks external links, loses slug readability, rewrites history
- **(B) Force all references to use new format** → Rejected: existing workplans, ADR bodies, operator memory use legacy IDs; mass-edit brittle
- **(C) Maintain manual mapping doc** → Rejected: drift-prone, not machine-readable, doesn't integrate with tooling

**Decision**: Build a **bidirectional alias table** at `docs/adrs/aliases.json` mapping legacy timestamp IDs to canonical filename-derived IDs. Update all ID-extraction code paths to consult the alias table and accept **both formats forever**, normalizing to canonical for display.

## Decision

### 1. Alias Table Schema

`docs/adrs/aliases.json`:
```json
{
  "2603221500": "ADR-2026-03-22-timestamp-adr-numbering",
  "2605082500": "ADR-2026-05-08-typed-tool-library-and-sop-execution"
}
```
- **Keys**: 10-digit timestamp (no `ADR-` prefix)
- **Values**: full canonical ID including `ADR-` prefix and slug
- **Generation**: `hex adr aliases generate` scans `docs/adrs/ADR-*.md`, parses `ADR-YYMMDDHHMM-<slug>.md` filenames, derives `ADR-YYYY-MMDD-<slug>` by expanding year (20YY for YY < 70, 19YY otherwise)

### 2. CLI Subcommand

Add `hex adr aliases` with actions:
```
hex adr aliases generate       # Scan docs/adrs/, write/update aliases.json
hex adr aliases show           # Pretty-print the alias table
```

Implementation: `hex-cli/src/commands/adr/mod.rs`:
```rust
pub enum AdrAction {
    // ... existing variants ...
    Aliases {
        #[command(subcommand)]
        action: AliasAction,
    },
}

pub enum AliasAction {
    Generate,
    Show,
}
```

`generate`:
1. Scan `docs/adrs/` for files matching `ADR-(\d{10})-(.+)\.md`
2. For each match, extract `(timestamp, slug)`
3. Parse `YY` prefix; expand to `YYYY` (20YY if YY < 70 else 19YY)
4. Build canonical ID: `ADR-YYYY-MM-DD-{slug}`
5. Write/merge with existing `aliases.json` (preserve manual entries)

`show`:
- Read `aliases.json`, render as table with columns: Legacy | Canonical | Filename

### 3. Parser Integration

**A. `hex-cli/src/commands/adr/mod.rs`** — update `extract_adr_id(filename: &str)`:

```rust
fn extract_adr_id(filename: &str) -> String {
    // Existing: strip "ADR-" prefix, take leading digits → "ADR-{digits}"
    // NEW: after extracting timestamp, consult aliases.json; if found, return canonical
    if let Some(rest) = filename.strip_prefix("ADR-").or_else(|| filename.strip_prefix("adr-")) {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            // Check alias table
            if let Some(canonical) = lookup_alias(&digits) {
                return canonical;
            }
            return format!("ADR-{}", digits);
        }
    }
    filename.to_string()
}

fn lookup_alias(timestamp: &str) -> Option<String> {
    // Lazy-load docs/adrs/aliases.json into static once_cell HashMap
    // Return aliases.get(timestamp).cloned()
    // If file missing or parse error, return None (fail-open)
}
```

Also update `search()` to accept both forms: normalize query via alias lookup before filtering.

**B. `hex-nexus/src/orchestration/workplan_auto_emitter.rs:93`** — coverage scan loop:

```rust
// Existing (line 117-127):
let mut i = 0;
while let Some(idx) = content[i..].find("ADR-") {
    let abs = i + idx + 4;
    let tail = &content[abs..];
    let end = tail.find(|c: char| !c.is_ascii_digit()).unwrap_or(tail.len());
    if end >= 8 {
        covered_adr_ids.insert(tail[..end].to_string());
    }
    i = abs + 1;
}
// NEW: also check for "ADR-YYYY-MM-DD-<slug>" pattern and reverse-lookup via aliases.json
```

Add parallel scan for canonical pattern:
```rust
// After existing digit-only scan, add:
let re = regex::Regex::new(r"ADR-\d{4}-\d{2}-\d{2}-[\w-]+").unwrap();
for cap in re.captures_iter(&content) {
    let canonical = cap.get(0).unwrap().as_str();
    if let Some(timestamp) = reverse_lookup_alias(canonical) {
        covered_adr_ids.insert(timestamp);
    }
}
```

Helper:
```rust
fn reverse_lookup_alias(canonical: &str) -> Option<String> {
    // Load aliases.json, invert map, return timestamp for canonical ID
    // Lazy-load into static once_cell HashMap<String, String>
}
```

**C. `hex-nexus/src/tools/adr_draft.rs:98`** — filename construction:

Existing:
```rust
let target_path = format!("docs/adrs/ADR-{}-{}.md", id, slug);
```

NEW: after file lands, update aliases.json via proposed_action or post-hook.

**D. `hex-nexus/src/tools/adr_status_set.rs:62`** — glob resolution:

Existing:
```rust
if !adr_id.chars().all(|c| c.is_ascii_digit()) || adr_id.len() < 8 {
    return ToolResult::err("adr_id must be a digits-only timestamp ≥8 chars", ...);
}
```

NEW: also accept canonical `ADR-YYYY-MM-DD-slug` format; normalize to timestamp for glob pattern `ADR-{timestamp}-*.md`.

### 4. Backward Compatibility

- **Zero file renames**: aliases.json is overlay only
- **Both formats valid forever**: legacy `ADR-2603221500` and canonical `ADR-2026-03-22-timestamp-adr-numbering` both resolve to same file
- **Tooling gracefully degrades**: if aliases.json missing or corrupt, tools fall back to digit-only extraction (current behavior)
- **Display preference**: tools emit canonical ID when available; fallback to `ADR-{timestamp}` if no alias

### 5. Rollout

1. Ship `hex adr aliases generate` (P0.1)
2. Run `hex adr aliases generate` once to seed aliases.json (P0.2)
3. Update `extract_adr_id()` and workplan_auto_emitter lookup (P1.1–P1.2)
4. Update `adr_draft` and `adr_status_set` (P2.1–P2.2)
5. Verify via `cargo check` and integration test `hex adr search ADR-2603221500` (P3.1)

## Consequences

### Positive

- **Operator UX**: legacy IDs in workplan JSON and memory notes remain valid; no mass-edit needed
- **Content-filter resilience**: canonical IDs with slugs avoid consecutive-digit triggers
- **Discoverability**: `hex adr aliases show` bridges legacy references to readable filenames
- **Future-proof**: new ADRs auto-add to aliases.json via `generate` (can run periodically or post-commit hook)
- **Zero history rewrite**: git log and external links unaffected

### Negative

- **Maintenance burden**: `aliases.json` must stay in sync with `docs/adrs/` directory; stale entries if file renamed manually
  - *Mitigation*: `hex adr doctor` warns on orphaned aliases (file missing) or missing aliases (ADR file exists but not in table)
- **Lookup latency**: alias table adds lazy-load + HashMap lookup to every `extract_adr_id()` call
  - *Mitigation*: static `once_cell` HashMap — amortized cost near-zero after first load
- **Two sources of truth**: aliases.json duplicates data already in filenames
  - *Mitigation*: `aliases generate` is idempotent; can regenerate from filenames anytime

### Risks

- **Alias conflicts**: if two ADRs have same timestamp (collision handled by ADR-2603221500 via alpha suffix like `2603221500a`)
  - *Mitigation*: `generate` command warns on collision; operator manually adds `a` suffix to aliases.json key
- **Canonical ID ambiguity**: if slug changes via manual file rename, aliases.json points to old slug
  - *Mitigation*: `hex adr doctor` detects this as "Dangling alias — file not found" and proposes fix

### Dependencies

- `hex-cli`: add `serde_json` dep (already present) + `regex` crate for canonical ID pattern matching (already in Cargo.toml per ADR-2604142200)
- `hex-nexus`: add `regex` dep + `once_cell` for lazy-loaded alias map
- `docs/adrs/aliases.json`: new file, tracked in git

### Open Questions

- **Should `hex adr aliases generate` run automatically post-ADR-creation?**
  - Proposal: add to `adr_draft` tool as final step (writes aliases.json entry via proposed_action) OR add git post-commit hook
  - Decision deferred to implementation phase (P0)

- **Pluralization: `hex adr aliases` vs `hex adr alias`?**
  - Precedent: `hex secrets` (plural resource), `hex adr abandoned` (plural query)
  - Decision: `hex adr aliases` (plural) matches pattern

## Implementation Notes

**File**: `hex-cli/src/commands/adr/aliases.rs` (new module)

**Lazy-load pattern**:
```rust
use once_cell::sync::Lazy;
use std::collections::HashMap;

static ALIASES: Lazy<HashMap<String, String>> = Lazy::new(|| {
    let path = find_adr_dir()?.join("aliases.json");
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
});
```

**ADR doctor integration** (P3):
- Add `AliasOrphaned` finding kind (alias → file missing)
- Add `AliasStale` finding kind (ADR file exists, not in aliases.json)
- Tier-A auto-fix: regenerate aliases.json via `hex adr aliases generate` on shadow branch

## References

- ADR-2603221500: Timestamp-Based ADR Numbering (original decision)
- ADR-2605082500: Typed Tool Library + SOP Execution (defines tool contract)
- Operator msg 56ea88ae: re-fire trigger after content-filter collision
- `hex-cli/src/commands/adr/mod.rs:449`: `extract_adr_id()` implementation
- `hex-nexus/src/orchestration/workplan_auto_emitter.rs:93`: workplan coverage scan

## Acceptance Criteria

1. `cargo check` passes workspace-wide
2. `hex adr aliases generate` creates `docs/adrs/aliases.json` with ≥1 entry per existing timestamp-based ADR
3. `hex adr search ADR-2603221500` and `hex adr search ADR-2026-03-22-timestamp-adr-numbering` both resolve to same ADR file
4. Workplan auto-emitter coverage scan recognizes both legacy and canonical ID formats in workplan JSON
5. `hex adr doctor` detects orphaned and missing aliases (if `--json`, includes in findings array)
