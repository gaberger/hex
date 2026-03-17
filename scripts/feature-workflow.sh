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

  # Parse optional flags
  shift
  while [ $# -gt 0 ]; do
    case "$1" in
      --force) force_merge=true ;;
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

    if git merge "$branch" --no-ff -m "feat(${feature_name}): merge ${component}"; then
      log_ok "Merged $branch successfully"
      merged=$((merged + 1))
    else
      log_err "Merge conflict on $branch — aborting merge"
      git merge --abort
      failed=$((failed + 1))
      log_warn "Attempting rebase of $branch onto main..."

      if git rebase main "$branch" && git checkout main && git merge "$branch" --no-ff -m "feat(${feature_name}): merge ${component}"; then
        log_ok "Rebase + merge of $branch succeeded"
        merged=$((merged + 1))
        failed=$((failed - 1))
      else
        git rebase --abort 2>/dev/null || true
        git checkout main 2>/dev/null || true
        log_err "Cannot merge $branch — manual resolution needed"
      fi
    fi
  done

  echo ""
  log_info "Merge complete: $merged succeeded, $failed failed"

  if [ "$failed" -eq 0 ]; then
    # Run full gate suite
    log_info "Running full gate suite..."
    bun run check && bun test && bun run lint
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
    echo "  $0 merge <feature-name> [--force]  Merge worktrees (warns without validation report)"
    echo "  $0 cleanup <feature-name>   Remove worktrees and branches"
    echo "  $0 list                     List all feature worktrees"
    echo "  $0 stale [hours]            Find worktrees with no recent commits (default: 24h)"
    ;;
esac
