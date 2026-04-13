#!/usr/bin/env bash
# P2 — CI drift detection: every pub trait I*Port in hex-core/src/ports/
# must have a corresponding ## Sigma_* section in docs/algebra/ports-signature.md.
#
# Usage: ./scripts/check-ports-signature.sh
# Exit 0 = in sync, Exit 1 = drift detected
#
# ADR-2604111229 Phase 2

set -euo pipefail

PORTS_DIR="hex-core/src/ports"
ALGEBRA_DOC="docs/algebra/ports-signature.md"

# Extract trait names from source
traits=$(grep -rh "pub trait I.*Port" "$PORTS_DIR" \
    | sed 's/.*pub trait //' \
    | sed 's/[:{< ].*//' \
    | sort)

drift=0
missing_in_doc=()
missing_in_source=()

# Check: every trait in source has a section in the algebra doc
for trait in $traits; do
    if ! grep -q "$trait" "$ALGEBRA_DOC" 2>/dev/null; then
        missing_in_doc+=("$trait")
        drift=1
    fi
done

# Check: every Sigma_* section in the doc maps to a real trait
doc_traits=$(grep '^## Sigma_' "$ALGEBRA_DOC" 2>/dev/null \
    | sed 's/.*-- //' \
    | sort)
for doc_trait in $doc_traits; do
    if ! echo "$traits" | grep -q "$doc_trait"; then
        missing_in_source+=("$doc_trait")
        drift=1
    fi
done

if [ $drift -eq 0 ]; then
    echo "ports-signature: OK — all $( echo "$traits" | wc -l | tr -d ' ') traits in sync"
    exit 0
fi

if [ ${#missing_in_doc[@]} -gt 0 ]; then
    echo "DRIFT: traits in source but missing from $ALGEBRA_DOC:"
    for t in "${missing_in_doc[@]}"; do
        echo "  - $t"
    done
fi

if [ ${#missing_in_source[@]} -gt 0 ]; then
    echo "DRIFT: traits in $ALGEBRA_DOC but missing from source:"
    for t in "${missing_in_source[@]}"; do
        echo "  - $t"
    done
fi

exit 1
