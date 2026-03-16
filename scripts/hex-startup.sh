#!/bin/bash
# hex session-start hook
# Reads PRD.md and scans project structure to present startup context

set -e

# Only run in hex projects (check for CLAUDE.md + PRD.md)
if [ ! -f "PRD.md" ] || [ ! -f "CLAUDE.md" ]; then
  exit 0
fi

echo ""
echo "=== hex Project Detected ==="
echo ""

# Extract project name and summary from PRD.md
NAME=$(head -1 PRD.md | sed 's/^# //' | sed 's/ —.*//')
SUMMARY=$(awk '/^## Summary/{found=1;next} /^##/{found=0} found && NF && !/^_/' PRD.md | head -1)

echo "Project: $NAME"
if [ -n "$SUMMARY" ]; then
  echo "Goal: $SUMMARY"
fi
echo ""

# Detect stack type
if [ -d "backend" ] && [ -d "frontend" ]; then
  echo "Stack: Multi-stack"
  echo "  backend/  — $(ls backend/src/core/domain/ 2>/dev/null | grep -v gitkeep | wc -l | tr -d ' ') domain files"
  echo "  frontend/ — $(ls frontend/src/core/domain/ 2>/dev/null | grep -v gitkeep | wc -l | tr -d ' ') domain files"
elif [ -d "src/core" ]; then
  echo "Stack: Single-stack"
fi
echo ""

# Assess progress through hex pipeline
echo "Pipeline Status:"

check_layer() {
  local dir="$1"
  local label="$2"
  local base="${3:-.}"
  local path="$base/src/core/$dir"
  [ -d "$base/src/adapters/$dir" ] && path="$base/src/adapters/$dir"

  local count=$(find "$path" -name '*.ts' -o -name '*.go' -o -name '*.rs' 2>/dev/null | grep -v gitkeep | wc -l | tr -d ' ')
  if [ "$count" -gt 0 ]; then
    echo "  [done] $label ($count files)"
  else
    echo "  [todo] $label"
  fi
}

BASE="."
if [ -d "backend" ]; then
  echo " Backend:"
  BASE="backend"
fi

# Check each hex layer
DOMAIN_FILES=$(find "$BASE/src/core/domain" -name '*.ts' -o -name '*.go' -o -name '*.rs' 2>/dev/null | grep -v gitkeep | wc -l | tr -d ' ')
PORTS_FILES=$(find "$BASE/src/core/ports" -name '*.ts' -o -name '*.go' -o -name '*.rs' 2>/dev/null | grep -v gitkeep | wc -l | tr -d ' ')
UC_FILES=$(find "$BASE/src/core/usecases" -name '*.ts' -o -name '*.go' -o -name '*.rs' 2>/dev/null | grep -v gitkeep | wc -l | tr -d ' ')
PRI_FILES=$(find "$BASE/src/adapters/primary" -name '*.ts' -o -name '*.go' -o -name '*.rs' 2>/dev/null | grep -v gitkeep | wc -l | tr -d ' ')
SEC_FILES=$(find "$BASE/src/adapters/secondary" -name '*.ts' -o -name '*.go' -o -name '*.rs' 2>/dev/null | grep -v gitkeep | wc -l | tr -d ' ')
TEST_FILES=$(find "$BASE/tests" -name '*.test.*' -o -name '*_test.*' 2>/dev/null | wc -l | tr -d ' ')

status() { [ "$1" -gt 0 ] && echo "done" || echo "todo"; }

echo "  [$(status $DOMAIN_FILES)] Domain ($DOMAIN_FILES files)"
echo "  [$(status $PORTS_FILES)] Ports ($PORTS_FILES files)"
echo "  [$(status $UC_FILES)] Use Cases ($UC_FILES files)"
echo "  [$(status $PRI_FILES)] Primary Adapters ($PRI_FILES files)"
echo "  [$(status $SEC_FILES)] Secondary Adapters ($SEC_FILES files)"
echo "  [$(status $TEST_FILES)] Tests ($TEST_FILES files)"

if [ -d "frontend" ]; then
  echo ""
  echo " Frontend:"
  FE_DOMAIN=$(find "frontend/src/core/domain" -name '*.ts' 2>/dev/null | grep -v gitkeep | wc -l | tr -d ' ')
  FE_PORTS=$(find "frontend/src/core/ports" -name '*.ts' 2>/dev/null | grep -v gitkeep | wc -l | tr -d ' ')
  FE_UC=$(find "frontend/src/core/usecases" -name '*.ts' 2>/dev/null | grep -v gitkeep | wc -l | tr -d ' ')
  FE_PRI=$(find "frontend/src/adapters/primary" -name '*.ts' 2>/dev/null | grep -v gitkeep | wc -l | tr -d ' ')
  FE_SEC=$(find "frontend/src/adapters/secondary" -name '*.ts' 2>/dev/null | grep -v gitkeep | wc -l | tr -d ' ')
  FE_TEST=$(find "frontend/tests" -name '*.test.*' 2>/dev/null | wc -l | tr -d ' ')

  echo "  [$(status $FE_DOMAIN)] Domain ($FE_DOMAIN files)"
  echo "  [$(status $FE_PORTS)] Ports ($FE_PORTS files)"
  echo "  [$(status $FE_UC)] Use Cases ($FE_UC files)"
  echo "  [$(status $FE_PRI)] Primary Adapters ($FE_PRI files)"
  echo "  [$(status $FE_SEC)] Secondary Adapters ($FE_SEC files)"
  echo "  [$(status $FE_TEST)] Tests ($FE_TEST files)"
fi

# Determine next step
echo ""
if [ "$DOMAIN_FILES" -eq 0 ]; then
  echo "Next step: Define domain entities and value objects"
elif [ "$PORTS_FILES" -eq 0 ]; then
  echo "Next step: Define port interfaces (contracts)"
elif [ "$UC_FILES" -eq 0 ]; then
  echo "Next step: Implement use cases"
elif [ "$PRI_FILES" -eq 0 ] && [ "$SEC_FILES" -eq 0 ]; then
  echo "Next step: Implement adapters"
elif [ "$TEST_FILES" -eq 0 ]; then
  echo "Next step: Add tests"
else
  echo "Next step: Run 'hex analyze .' to validate architecture"
fi

echo ""
echo "Type what you'd like to work on, or press Enter to follow the pipeline."
echo "================================"
