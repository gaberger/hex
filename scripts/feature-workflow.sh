#!/usr/bin/env bash
# feature-workflow.sh — Worktree lifecycle management for hex feature development
#
# Usage:
#   ./scripts/feature-workflow.sh setup <feature-name> [--skip-specs]  Create worktrees from workplan
#   ./scripts/feature-workflow.sh status <feature-name>     Show worktree status
#   ./scripts/feature-workflow.sh merge <feature-name> [--force]  Merge worktrees in dependency order
#   ./scripts/feature-workflow.sh cleanup <feature-name>    Remove worktrees and branches
#   ./scripts/feature-workflow.sh list                      List all feature worktrees
#   ./scripts/feature-workflow.sh stale                     Find worktrees with no recent commits

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
WORKTREE_BASE="$(dirname "$PROJECT_ROOT")"
MAX_WORKTREES=8

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
NC='\033[0m'

log_info()  { echo -e "${BLUE}[info]${NC} $*"; }
log_ok()    { echo -e "${GREEN}[ok]${NC} $*"; }
log_warn()  { echo -e "${YELLOW}[warn]${NC} $*"; }
log_err()   { echo -e "${RED}[error]${NC} $*"; }

# Cargo manifest path: respect HEX_FEATURE_CARGO_MANIFEST override for sub-projects
# (e.g. examples/ebay-clone/backend/Cargo.toml).
cargo_manifest_path() {
  if [ -n "${HEX_FEATURE_CARGO_MANIFEST:-}" ]; then
    echo "$HEX_FEATURE_CARGO_MANIFEST"
  elif [ -f "$PROJECT_ROOT/Cargo.toml" ]; then
    echo "$PROJECT_ROOT/Cargo.toml"
  fi
}

# Count cargo errors. Returns "0" if cargo unavailable or no manifest.
# Never fails — caller can rely on the integer in stdout.
cargo_check_errors() {
  local manifest
  manifest=$(cargo_manifest_path)
  if [ -z "$manifest" ] || ! command -v cargo >/dev/null 2>&1; then
    echo "0"
    return
  fi
  local output
  output=$(cargo check --workspace --message-format=short --manifest-path "$manifest" 2>&1 || true)
  # Prefer cargo's authoritative summary "due to N previous error(s)" — one
  # short-format line can bundle multiple errors under a single error[Exxxx],
  # so line-counting under-reports. Fall back to line count if no summary
  # (e.g. partial compile before frontend error).
  local count
  count=$(echo "$output" | grep -oE "due to [0-9]+ previous error" | grep -oE "[0-9]+" | tail -1)
  if [ -z "$count" ]; then
    count=$(echo "$output" | grep -cE "error\[E[0-9]+\]" || true)
  fi
  echo "$count"
}

# List source files with cargo errors (sorted unique, repo-relative paths).
# Excludes files that only have warnings — drift detection cares about errors.
cargo_check_errored_files() {
  local manifest
  manifest=$(cargo_manifest_path)
  if [ -z "$manifest" ] || ! command -v cargo >/dev/null 2>&1; then
    return
  fi
  local output
  output=$(cargo check --workspace --message-format=short --manifest-path "$manifest" 2>&1 || true)
  echo "$output" \
    | grep -E "error\[E[0-9]+\]" \
    | grep -oE "[^[:space:]]+\.rs(:[0-9]+)?" \
    | sed 's/:[0-9]*$//' \
    | sort -u
}

#--- setup: Create worktrees from a workplan ---
cmd_setup() {
  local feature_name="$1"
  local skip_specs=false

  # Parse optional flags
  shift
  while [ $# -gt 0 ]; do
    case "$1" in
      --skip-specs) skip_specs=true ;;
      *) log_warn "Unknown flag: $1" ;;
    esac
    shift
  done

  # Enforce specs-first pipeline: block if no behavioral spec exists
  local spec_file="$PROJECT_ROOT/docs/specs/${feature_name}.json"
  if [ ! -f "$spec_file" ]; then
    if [ "$skip_specs" = true ]; then
      log_warn "Skipping spec check (--skip-specs). Specs-first pipeline bypassed."
    else
      log_err "Behavioral spec not found: $spec_file"
      log_info "The specs-first pipeline requires behavioral specs before worktree setup."
      log_info "Create specs by running the behavioral-spec-writer agent, or use:"
      log_info "  $0 setup $feature_name --skip-specs"
      log_info "to bypass this check (for hotfixes only)."
      exit 1
    fi
  else
    log_ok "Behavioral spec found: $spec_file"
  fi

  local workplan="$PROJECT_ROOT/docs/workplans/feat-${feature_name}.json"

  # Check for behavioral spec (warn but don't block)
  local specfile="$PROJECT_ROOT/docs/specs/${feature_name}.json"
  if [ ! -f "$specfile" ]; then
    log_warn "No behavioral spec found at docs/specs/${feature_name}.json — consider running specs phase first"
  fi

  if [ ! -f "$workplan" ]; then
    log_err "Workplan not found: $workplan"
    log_info "Run the planner agent first to generate the workplan."
    exit 1
  fi

  # Check current worktree count
  local current_count
  current_count=$(git -C "$PROJECT_ROOT" worktree list | wc -l | tr -d ' ')
  if [ "$current_count" -ge "$MAX_WORKTREES" ]; then
    log_err "Too many worktrees ($current_count >= $MAX_WORKTREES). Clean up stale worktrees first."
    exit 1
  fi

  log_info "Setting up worktrees for feature: $feature_name"

  # Extract step IDs and adapter names from workplan
  # Expected format: steps[].worktree_branch
  local branches
  branches=$(python3 -c "
import json, sys
with open('$workplan') as f:
    wp = json.load(f)
for step in wp.get('steps', []):
    branch = step.get('worktree_branch', '')
    if branch:
        # Extract the last segment as the worktree dir name
        parts = branch.split('/')
        dirname = '-'.join(parts[1:])  # e.g. feature-name/adapter -> feature-name-adapter
        print(f'{branch}|{dirname}')
" 2>/dev/null || true)

  if [ -z "$branches" ]; then
    log_warn "No worktree branches found in workplan. Creating default structure."
    # Default structure: domain, ports, integration
    branches="feat/${feature_name}/domain|feat-${feature_name}-domain
feat/${feature_name}/ports|feat-${feature_name}-ports
feat/${feature_name}/integration|feat-${feature_name}-integration"
  fi

  local created=0
  while IFS='|' read -r branch dirname; do
    local worktree_path="$WORKTREE_BASE/hex-${dirname}"

    if git -C "$PROJECT_ROOT" worktree list | grep -q "$worktree_path"; then
      log_warn "Worktree already exists: $worktree_path (skipping)"
      continue
    fi

    # Create branch from current HEAD if it doesn't exist
    if ! git -C "$PROJECT_ROOT" rev-parse --verify "$branch" >/dev/null 2>&1; then
      git -C "$PROJECT_ROOT" branch "$branch" HEAD
    fi

    git -C "$PROJECT_ROOT" worktree add "$worktree_path" "$branch"
    log_ok "Created worktree: $worktree_path → $branch"
    created=$((created + 1))
  done <<< "$branches"

  log_info "Created $created worktrees for feature: $feature_name"
  echo ""
  cmd_status "$feature_name"
}

#--- status: Show worktree status for a feature ---
cmd_status() {
  local feature_name="$1"

  log_info "Feature: $feature_name"
  echo ""
  printf "%-45s %-35s %-10s\n" "WORKTREE" "BRANCH" "COMMITS"
  printf "%-45s %-35s %-10s\n" "--------" "------" "-------"

  git -C "$PROJECT_ROOT" worktree list --porcelain | while read -r line; do
    if [[ "$line" == worktree\ * ]]; then
      local wt_path="${line#worktree }"
      local branch=""
      local commits=0

      # Read the branch line
      read -r line2 || true
      read -r line3 || true
      if [[ "$line3" == branch\ * ]]; then
        branch="${line3#branch refs/heads/}"
      fi
      read -r _ || true  # blank line

      # Filter to this feature's worktrees
      if [[ "$branch" == *"$feature_name"* ]]; then
        # Count commits ahead of main
        commits=$(git -C "$PROJECT_ROOT" rev-list --count "main..$branch" 2>/dev/null || echo "0")
        local short_path="${wt_path/#$WORKTREE_BASE\//}"
        printf "%-45s %-35s %-10s\n" "$short_path" "$branch" "$commits"
      fi
    fi
  done

  echo ""
  # Show specs and workplan status
  if [ -f "$PROJECT_ROOT/docs/specs/${feature_name}.json" ]; then
    local spec_count
    spec_count=$(python3 -c "import json; print(len(json.load(open('$PROJECT_ROOT/docs/specs/${feature_name}.json'))))" 2>/dev/null || echo "?")
    log_ok "Behavioral specs: $spec_count specs in docs/specs/${feature_name}.json"
  else
    log_warn "Behavioral specs: NOT FOUND (run behavioral-spec-writer first)"
  fi

  if [ -f "$PROJECT_ROOT/docs/workplans/feat-${feature_name}.json" ]; then
    local step_count
    step_count=$(python3 -c "import json; print(len(json.load(open('$PROJECT_ROOT/docs/workplans/feat-${feature_name}.json')).get('steps', [])))" 2>/dev/null || echo "?")
    log_ok "Workplan: $step_count steps in docs/workplans/feat-${feature_name}.json"
  else
    log_warn "Workplan: NOT FOUND (run planner agent first)"
  fi
}

#--- merge: Merge worktrees in dependency order ---
cmd_merge() {
  local feature_name="$1"
  local force_merge=false
  local no_rollback=false
  local no_cargo_gate=false

  # Parse optional flags
  shift
  while [ $# -gt 0 ]; do
    case "$1" in
      --force) force_merge=true ;;
      --no-rollback) no_rollback=true ;;
      --no-cargo-gate) no_cargo_gate=true ;;
      *) log_warn "Unknown flag: $1" ;;
    esac
    shift
  done

  # Check for validation report
  local validation_report="$PROJECT_ROOT/docs/validation/${feature_name}.json"
  if [ ! -f "$validation_report" ]; then
    log_warn "No validation report found at: $validation_report"
    log_info "Run the validation-judge agent before merging to ensure specs are satisfied."
    if [ "$force_merge" = true ]; then
      log_warn "Proceeding without validation report (--force)."
    else
      echo -n "Continue without validation report? [y/N] "
      read -r confirm
      if [[ ! "$confirm" =~ ^[Yy]$ ]]; then
        log_info "Merge aborted. Run validation-judge first, or use: $0 merge $feature_name --force"
        exit 1
      fi
    fi
  else
    log_ok "Validation report found: $validation_report"
  fi

  local workplan="$PROJECT_ROOT/docs/workplans/feat-${feature_name}.json"

  log_info "Merging worktrees for feature: $feature_name"

  # Dependency order: domain → ports → secondary → primary → usecases → integration
  local merge_order=("domain" "ports")

  # Extract adapter names from workplan if available
  if [ -f "$workplan" ]; then
    local adapters
    adapters=$(python3 -c "
import json
with open('$workplan') as f:
    wp = json.load(f)
for step in wp.get('steps', []):
    layer = step.get('layer', '')
    adapter = step.get('adapter', '')
    if adapter and adapter not in ('domain', 'ports', 'integration'):
        prefix = '1' if 'secondary' in layer else '2'
        print(f'{prefix}|{adapter}')
" 2>/dev/null | sort | cut -d'|' -f2 || true)

    while IFS= read -r adapter; do
      [ -n "$adapter" ] && merge_order+=("$adapter")
    done <<< "$adapters"
  fi

  merge_order+=("integration")

  # Ensure we're on main
  cd "$PROJECT_ROOT"
  local current_branch
  current_branch=$(git branch --show-current)
  if [ "$current_branch" != "main" ]; then
    log_err "Must be on main branch to merge. Currently on: $current_branch"
    exit 1
  fi

  local merged=0
  local failed=0
  local rolled_back=0

  # §5.8 post-merge cargo-check gate: capture baseline before any merges.
  # Detect drift by comparing per-merge error count + which files now have errors.
  local cargo_gate_active=false
  local manifest
  manifest=$(cargo_manifest_path)
  if [ "$no_cargo_gate" = false ] && [ -n "$manifest" ] && command -v cargo >/dev/null 2>&1; then
    cargo_gate_active=true
    log_info "Cargo-check gate active (manifest: ${manifest#$PROJECT_ROOT/})"
    log_info "Running pre-merge cargo check baseline..."
  fi
  local pre_merge_errors
  pre_merge_errors=$(cargo_check_errors)
  local baseline_errors=$pre_merge_errors
  if [ "$cargo_gate_active" = true ]; then
    log_info "Baseline cargo errors: $baseline_errors"
  fi

  for component in "${merge_order[@]}"; do
    local branch="feat/${feature_name}/${component}"

    # Check if branch exists
    if ! git rev-parse --verify "$branch" >/dev/null 2>&1; then
      log_warn "Branch $branch does not exist (skipping)"
      continue
    fi

    # Check if there are commits to merge
    local ahead
    ahead=$(git rev-list --count "main..$branch" 2>/dev/null || echo "0")
    if [ "$ahead" -eq 0 ]; then
      log_warn "Branch $branch has no commits ahead of main (skipping)"
      continue
    fi

    log_info "Merging $branch ($ahead commits)..."

    local merge_succeeded=false
    if git merge "$branch" --no-ff -m "feat(${feature_name}): merge ${component}"; then
      log_ok "Merged $branch successfully"
      merge_succeeded=true
    else
      log_err "Merge conflict on $branch — aborting merge"
      git merge --abort
      log_warn "Attempting rebase of $branch onto main..."

      if git rebase main "$branch" && git checkout main && git merge "$branch" --no-ff -m "feat(${feature_name}): merge ${component}"; then
        log_ok "Rebase + merge of $branch succeeded"
        merge_succeeded=true
      else
        git rebase --abort 2>/dev/null || true
        git checkout main 2>/dev/null || true
        log_err "Cannot merge $branch — manual resolution needed"
        failed=$((failed + 1))
        continue
      fi
    fi

    # §5.8 gate: run cargo check after a successful merge, detect drift.
    if [ "$cargo_gate_active" = true ] && [ "$merge_succeeded" = true ]; then
      log_info "Running post-merge cargo check for $branch..."
      local post_errors
      post_errors=$(cargo_check_errors)
      local delta=$((post_errors - pre_merge_errors))

      if [ "$delta" -le 0 ]; then
        log_ok "Cargo errors: $pre_merge_errors → $post_errors (no regression from $component)"
        merged=$((merged + 1))
        pre_merge_errors=$post_errors
        continue
      fi

      # Errors went up. Classify: drift (errors in files NOT touched by this merge)
      # vs intentional (errors in files THIS merge edited).
      local merged_files drift_files
      merged_files=$(git diff --name-only "HEAD~1" "HEAD" 2>/dev/null | grep '\.rs$' | sort -u || true)
      local current_errored
      current_errored=$(cargo_check_errored_files)
      # Drift = errored files NOT in this merge's changeset
      drift_files=$(comm -23 \
        <(printf '%s\n' "$current_errored" | sort -u) \
        <(printf '%s\n' "$merged_files" | sort -u) 2>/dev/null || true)

      log_warn "Cargo errors increased: $pre_merge_errors → $post_errors (delta +$delta) after merging $component"
      if [ -n "$drift_files" ]; then
        log_warn "Drift detected — errors in files NOT modified by $component:"
        echo "$drift_files" | sed 's/^/    /' | head -20
        if [ "$no_rollback" = false ]; then
          log_warn "Rolling back $component merge (use --no-rollback to disable)"
          git reset --hard "HEAD~1"
          rolled_back=$((rolled_back + 1))
          failed=$((failed + 1))
          continue
        else
          log_warn "Keeping merge despite drift (--no-rollback set)"
          merged=$((merged + 1))
          pre_merge_errors=$post_errors
        fi
      else
        log_info "Errors are in files THIS merge edited — intentional, not drift. Keeping merge."
        merged=$((merged + 1))
        pre_merge_errors=$post_errors
      fi
    else
      merged=$((merged + 1))
    fi
  done

  echo ""
  log_info "Merge complete: $merged succeeded, $failed failed, $rolled_back rolled back"

  if [ "$cargo_gate_active" = true ]; then
    local final_errors
    final_errors=$(cargo_check_errors)
    local final_delta=$((final_errors - baseline_errors))
    if [ "$final_delta" -gt 0 ]; then
      log_warn "Final cargo error count: $baseline_errors → $final_errors (+$final_delta from baseline)"
    elif [ "$final_delta" -lt 0 ]; then
      log_ok "Final cargo error count: $baseline_errors → $final_errors ($final_delta from baseline)"
    else
      log_ok "Final cargo error count: $final_errors (unchanged from baseline)"
    fi
  fi

  if [ "$failed" -eq 0 ]; then
    # Run full gate suite — only if a package.json is present at PROJECT_ROOT
    if [ -f "$PROJECT_ROOT/package.json" ]; then
      log_info "Running full gate suite (bun)..."
      bun run check && bun test && bun run lint
    fi
    if command -v hex >/dev/null 2>&1; then
      hex analyze .
    fi
    log_ok "All gates passed after merge"
  fi
}

#--- cleanup: Remove worktrees and branches ---
cmd_cleanup() {
  local feature_name="$1"

  log_info "Cleaning up worktrees for feature: $feature_name"

  local removed=0
  git -C "$PROJECT_ROOT" worktree list --porcelain | while read -r line; do
    if [[ "$line" == worktree\ * ]]; then
      local wt_path="${line#worktree }"
      local branch=""

      read -r _ || true
      read -r line3 || true
      if [[ "$line3" == branch\ * ]]; then
        branch="${line3#branch refs/heads/}"
      fi
      read -r _ || true

      if [[ "$branch" == *"$feature_name"* ]]; then
        log_info "Removing worktree: $wt_path"
        git -C "$PROJECT_ROOT" worktree remove "$wt_path" --force 2>/dev/null || true
        git -C "$PROJECT_ROOT" branch -d "$branch" 2>/dev/null || \
          git -C "$PROJECT_ROOT" branch -D "$branch" 2>/dev/null || true
        log_ok "Removed: $branch"
        removed=$((removed + 1))
      fi
    fi
  done

  # Prune any stale worktree references
  git -C "$PROJECT_ROOT" worktree prune
  log_info "Cleanup complete"
}

#--- list: List all feature worktrees ---
cmd_list() {
  log_info "All feature worktrees:"
  echo ""
  printf "%-45s %-35s %-10s %-20s\n" "WORKTREE" "BRANCH" "COMMITS" "LAST COMMIT"
  printf "%-45s %-35s %-10s %-20s\n" "--------" "------" "-------" "-----------"

  git -C "$PROJECT_ROOT" worktree list --porcelain | while read -r line; do
    if [[ "$line" == worktree\ * ]]; then
      local wt_path="${line#worktree }"
      local branch=""

      read -r _ || true
      read -r line3 || true
      if [[ "$line3" == branch\ * ]]; then
        branch="${line3#branch refs/heads/}"
      fi
      read -r _ || true

      if [[ "$branch" == feat/* ]]; then
        local commits
        commits=$(git -C "$PROJECT_ROOT" rev-list --count "main..$branch" 2>/dev/null || echo "0")
        local last_commit
        last_commit=$(git -C "$PROJECT_ROOT" log -1 --format="%ar" "$branch" 2>/dev/null || echo "unknown")
        local short_path="${wt_path/#$WORKTREE_BASE\//}"
        printf "%-45s %-35s %-10s %-20s\n" "$short_path" "$branch" "$commits" "$last_commit"
      fi
    fi
  done
}

#--- stale: Find worktrees with no recent commits ---
cmd_stale() {
  local stale_hours="${1:-24}"

  log_info "Worktrees with no commits in the last ${stale_hours} hours:"
  echo ""

  local stale_count=0
  local cutoff
  cutoff=$(date -v-${stale_hours}H +%s 2>/dev/null || date -d "${stale_hours} hours ago" +%s 2>/dev/null || echo "0")

  git -C "$PROJECT_ROOT" worktree list --porcelain | while read -r line; do
    if [[ "$line" == worktree\ * ]]; then
      local wt_path="${line#worktree }"
      local branch=""

      read -r _ || true
      read -r line3 || true
      if [[ "$line3" == branch\ * ]]; then
        branch="${line3#branch refs/heads/}"
      fi
      read -r _ || true

      if [[ "$branch" == feat/* ]]; then
        local last_ts
        last_ts=$(git -C "$PROJECT_ROOT" log -1 --format="%ct" "$branch" 2>/dev/null || echo "0")

        if [ "$last_ts" -lt "$cutoff" ]; then
          local last_commit
          last_commit=$(git -C "$PROJECT_ROOT" log -1 --format="%ar" "$branch" 2>/dev/null || echo "unknown")
          log_warn "$branch — last commit: $last_commit"
          stale_count=$((stale_count + 1))
        fi
      fi
    fi
  done

  if [ "$stale_count" -eq 0 ]; then
    log_ok "No stale worktrees found"
  fi
}

#--- Main ---
case "${1:-help}" in
  setup)    cmd_setup "${2:?Feature name required}" "${@:3}" ;;
  status)   cmd_status "${2:?Feature name required}" ;;
  merge)    cmd_merge "${2:?Feature name required}" "${@:3}" ;;
  cleanup)  cmd_cleanup "${2:?Feature name required}" ;;
  list)     cmd_list ;;
  stale)    cmd_stale "${2:-24}" ;;
  help|*)
    echo "hex feature-workflow — Worktree lifecycle for hex feature development"
    echo ""
    echo "Usage:"
    echo "  $0 setup <feature-name> [--skip-specs]  Create worktrees (blocks without specs)"
    echo "  $0 status <feature-name>    Show worktree and feature status"
    echo "  $0 merge <feature-name> [--force] [--no-rollback] [--no-cargo-gate]"
    echo "                              Merge worktrees in dependency order. Runs cargo check"
    echo "                              between each merge; rolls back merges that introduce"
    echo "                              errors in files they didn't touch (drift). Use"
    echo "                              --no-rollback to keep drifted merges; --no-cargo-gate"
    echo "                              to skip the per-merge check entirely. Set"
    echo "                              HEX_FEATURE_CARGO_MANIFEST=path/to/Cargo.toml to target"
    echo "                              a sub-project (e.g. examples/foo/backend/Cargo.toml)."
    echo "  $0 cleanup <feature-name>   Remove worktrees and branches"
    echo "  $0 list                     List all feature worktrees"
    echo "  $0 stale [hours]            Find worktrees with no recent commits (default: 24h)"
    ;;
esac
