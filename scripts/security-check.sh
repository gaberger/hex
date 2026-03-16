#!/usr/bin/env bash
# hex-intf Security Gate — Pre-commit hook
# Blocks on CRITICAL/HIGH findings, warns on MEDIUM/LOW
# Checks: path traversal, secrets, input validation, dependency audit

set -uo pipefail
# Note: NOT using set -e — grep returning non-zero (no match) is expected behavior

RED='\033[0;31m'
YELLOW='\033[0;33m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color
BOLD='\033[1m'

CRITICAL=0
HIGH=0
MEDIUM=0
LOW=0

fail() { echo -e "${RED}CRITICAL${NC}: $1"; ((CRITICAL++)); }
high() { echo -e "${RED}HIGH${NC}: $1"; ((HIGH++)); }
warn() { echo -e "${YELLOW}MEDIUM${NC}: $1"; ((MEDIUM++)); }
info() { echo -e "${YELLOW}LOW${NC}: $1"; ((LOW++)); }

echo -e "${BOLD}hex-intf Security Gate${NC}"
echo "────────────────────────────────"

# ── 1. Path Traversal ─────────────────────────────────
echo -e "\n${BOLD}[1/4] Path Traversal${NC}"

# Check that FileSystemAdapter uses safePath
if grep -rn 'path\.join.*filePath\|path\.resolve.*filePath' src/adapters/ --include="*.ts" | grep -v 'safePath\|resolve.*startsWith' | grep -v '\.test\.' > /dev/null 2>&1; then
  high "Adapter uses path.join/resolve with filePath without safePath() guard"
  grep -rn 'path\.join.*filePath\|path\.resolve.*filePath' src/adapters/ --include="*.ts" | grep -v 'safePath\|resolve.*startsWith' | grep -v '\.test\.' | head -3
else
  echo -e "  ${GREEN}OK${NC} — all file operations use safePath() or equivalent"
fi

# Check for .. in URL/path parameters without validation
if grep -rn 'req\.url\|req\.params\|decodeURIComponent' src/adapters/ --include="*.ts" | grep -v '\.\..*reject\|includes.*\.\.\|traversal' > /dev/null 2>&1; then
  # Only flag if there's no adjacent traversal check
  TRAVERSAL_FILES=$(grep -rln 'decodeURIComponent\|req\.url' src/adapters/ --include="*.ts" 2>/dev/null || true)
  for f in $TRAVERSAL_FILES; do
    if ! grep -q '\.\.' "$f" 2>/dev/null; then
      warn "File $f handles URL params but has no '..' traversal check"
    fi
  done
fi

# ── 2. Secret Detection ──────────────────────────────
echo -e "\n${BOLD}[2/4] Secret Detection${NC}"

# Staged files only (for pre-commit)
STAGED_FILES=$(git diff --cached --name-only --diff-filter=ACM 2>/dev/null || git diff --name-only 2>/dev/null || find src/ -name "*.ts" -type f)

SECRET_PATTERNS='(PRIVATE.KEY|sk-[a-zA-Z0-9]{20,}|ghp_[a-zA-Z0-9]{36}|password\s*=\s*["\x27][^"\x27]{8,}|api[_-]?key\s*=\s*["\x27][^"\x27]{8,}|-----BEGIN.*PRIVATE)'

FOUND_SECRETS=0
while IFS= read -r file; do
  if [[ -f "$file" ]] && [[ "$file" != *.lock ]] && [[ "$file" != *node_modules* ]]; then
    if grep -Pn "$SECRET_PATTERNS" "$file" 2>/dev/null; then
      fail "Potential secret in $file"
      FOUND_SECRETS=1
    fi
  fi
done <<< "$STAGED_FILES"

if [[ $FOUND_SECRETS -eq 0 ]]; then
  echo -e "  ${GREEN}OK${NC} — no secrets detected in staged files"
fi

# Check for .env files being committed
if echo "$STAGED_FILES" | grep -q '\.env$\|\.env\.local$\|\.env\.prod'; then
  fail ".env file staged for commit — secrets must not be committed"
fi

# ── 3. Input Validation ──────────────────────────────
echo -e "\n${BOLD}[3/4] Input Validation${NC}"

# Check primary adapters validate inputs at boundaries
PRIMARY_ADAPTERS=$(find src/adapters/primary -name "*.ts" -not -name "*.test.*" 2>/dev/null || true)
for adapter in $PRIMARY_ADAPTERS; do
  # Check if adapter has any user-facing input that's passed to ports without validation
  if grep -q 'req\.body\|argv\|req\.url' "$adapter" 2>/dev/null; then
    if ! grep -q 'typeof.*===\|\.length\|isNaN\|\.includes\|validate\|invalid\|error.*missing\|error.*required' "$adapter" 2>/dev/null; then
      high "Primary adapter $(basename $adapter) handles user input but has no visible validation"
    fi
  fi
done

# Check for unbounded reads
# Check each file with body reading for a size guard
BODY_READERS=$(grep -rln 'req\.on.*data\|readBody\|Buffer\.concat' src/adapters/ --include="*.ts" 2>/dev/null || true)
for f in $BODY_READERS; do
  if ! grep -q 'MAX_BODY\|maxSize\|size.*>\|\.length.*>' "$f" 2>/dev/null; then
    warn "File $(basename $f) reads request body without size limit"
  fi
done

echo -e "  ${GREEN}OK${NC} — primary adapter boundaries checked"

# ── 4. Dependency Audit ──────────────────────────────
echo -e "\n${BOLD}[4/4] Dependency Audit${NC}"

if command -v bun > /dev/null 2>&1; then
  # bun doesn't have built-in audit, check for known risky patterns
  if grep -q '@latest' src/ -r --include="*.ts" 2>/dev/null; then
    warn "Found @latest version specifier in source code — pins to unpredictable versions"
    grep -rn '@latest' src/ --include="*.ts" | head -3
  else
    echo -e "  ${GREEN}OK${NC} — no @latest version pins in source"
  fi
elif command -v npm > /dev/null 2>&1; then
  AUDIT_RESULT=$(npm audit --json 2>/dev/null || echo '{"vulnerabilities":{}}')
  VULN_COUNT=$(echo "$AUDIT_RESULT" | grep -c '"severity"' 2>/dev/null || echo "0")
  if [[ "$VULN_COUNT" -gt 0 ]]; then
    warn "$VULN_COUNT known vulnerabilities in dependencies (run 'npm audit' for details)"
  else
    echo -e "  ${GREEN}OK${NC} — no known vulnerabilities"
  fi
fi

# ── Summary ──────────────────────────────────────────
echo ""
echo "────────────────────────────────"
echo -e "${BOLD}Security Gate Summary${NC}"
echo -e "  CRITICAL: $CRITICAL  HIGH: $HIGH  MEDIUM: $MEDIUM  LOW: $LOW"

if [[ $CRITICAL -gt 0 ]] || [[ $HIGH -gt 0 ]]; then
  echo -e "\n${RED}${BOLD}BLOCKED${NC} — fix CRITICAL/HIGH findings before committing"
  exit 1
else
  if [[ $MEDIUM -gt 0 ]] || [[ $LOW -gt 0 ]]; then
    echo -e "\n${YELLOW}PASSED with warnings${NC} — review MEDIUM/LOW findings"
  else
    echo -e "\n${GREEN}${BOLD}PASSED${NC} — all security checks clean"
  fi
  exit 0
fi
