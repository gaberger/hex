#!/bin/bash
# hex session-start hook â presents project context on first prompt
set -e

# Only run in hex projects
[ ! -f "PRD.md" ] || [ ! -f "CLAUDE.md" ] && exit 0

echo ""
echo "=== hex Project ==="
echo ""

# Extract project info from PRD.md
NAME=$(head -1 PRD.md | sed 's/^# //' | sed 's/ â.*//')
SUMMARY=$(awk '/^## Summary/{f=1;next} /^##/{f=0} f && NF && !/^_/' PRD.md | head -1)
echo "Project: $NAME"
[ -n "$SUMMARY" ] && echo "Goal: $SUMMARY"
echo ""

# Check pipeline progress
BASE="."
[ -d "backend" ] && BASE="backend"

status() { [ "$1" -gt 0 ] && echo "done" || echo "todo"; }
count() { find "$1" \( -name "*.ts" -o -name "*.go" -o -name "*.rs" \) 2>/dev/null | grep -v gitkeep | wc -l | tr -d " "; }

D=$(count "$BASE/src/core/domain")
P=$(count "$BASE/src/core/ports")
U=$(count "$BASE/src/core/usecases")
PA=$(count "$BASE/src/adapters/primary")
SA=$(count "$BASE/src/adapters/secondary")
T=$(find "$BASE/tests" -name "*.test.*" -o -name "*_test.*" 2>/dev/null | wc -l | tr -d " ")

echo "Pipeline:"
echo "  [$(status $D)] Domain ($D)  [$(status $P)] Ports ($P)  [$(status $U)] UseCases ($U)"
echo "  [$(status $PA)] Primary ($PA)  [$(status $SA)] Secondary ($SA)  [$(status $T)] Tests ($T)"
echo ""

# Suggest next step
if [ "$D" -eq 0 ]; then echo "Next: Define domain entities in $BASE/src/core/domain/"
elif [ "$P" -eq 0 ]; then echo "Next: Define port interfaces in $BASE/src/core/ports/"
elif [ "$U" -eq 0 ]; then echo "Next: Implement use cases in $BASE/src/core/usecases/"
elif [ "$PA" -eq 0 ] && [ "$SA" -eq 0 ]; then echo "Next: Implement adapters"
elif [ "$T" -eq 0 ]; then echo "Next: Add tests"
else echo "Next: Run hex analyze . to validate"
fi
echo "==========================="
