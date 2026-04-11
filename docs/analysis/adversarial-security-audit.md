# Adversarial Security Audit — hex

**Date:** 2026-03-15
**Auditor:** Security Auditor Agent (claude-opus-4-6)
**Scope:** All adapter files, composition root, CLI, dashboard HTML, dashboard hub
**Method:** Manual static analysis of every source file in the attack surface

---

## Executive Summary

The hex codebase demonstrates strong security awareness: `execFile` over `exec`, a `safePath()` guard, `escapeHtml()`/`textContent` in the dashboard, body-size limits on HTTP endpoints, and API keys loaded exclusively from environment variables. However, the audit identified **4 confirmed vulnerabilities** and **3 items requiring manual review**.

| Severity | Count |
|----------|-------|
| HIGH     | 2     |
| MEDIUM   | 2     |
| LOW      | 0     |
| Needs Manual Review | 3 |

---

## Confirmed Vulnerabilities

### VULN-01: Path Traversal via Symlink Following (HIGH)

**File:** `src/adapters/secondary/filesystem-adapter.ts:60-66`

```typescript
private safePath(filePath: string): string {
  const abs = resolve(join(this.root, filePath));
  if (!abs.startsWith(this.root)) {
    throw new PathTraversalError(filePath, this.root);
  }
  return abs;
}
```

**Issue:** `safePath()` uses `resolve()` which operates on the lexical path only. It does NOT resolve symlinks. An attacker who can create a symlink inside the project root (e.g., `src/evil -> /etc`) can read arbitrary files via `fs.read('src/evil/passwd')` because:

1. `resolve(join(root, 'src/evil/passwd'))` produces `/project/src/evil/passwd`
2. This passes the `startsWith(root)` check
3. The OS follows the symlink to `/etc/passwd` at read time

**Attack vector:** Any code path where an LLM agent or user-controlled input creates a symlink inside the project directory, then reads through it. The `write()` method is also vulnerable -- symlinks could redirect writes to arbitrary locations.

**Remediation:** Use `fs.realpath()` on the resolved path before the prefix check:

```typescript
private async safePath(filePath: string): Promise<string> {
  const abs = resolve(join(this.root, filePath));
  if (!abs.startsWith(this.root)) {
    throw new PathTraversalError(filePath, this.root);
  }
  // Resolve symlinks THEN re-check
  const real = await realpath(abs);
  const realRoot = await realpath(this.root);
  if (!real.startsWith(realRoot)) {
    throw new PathTraversalError(filePath, this.root);
  }
  return real;
}
```

Note: This changes `safePath` from sync to async. All callers already use `await`.

---

### VULN-02: Error Message Information Disclosure (MEDIUM)

**Files:**
- `src/adapters/primary/dashboard-adapter.ts:184` -- exposes `err.message` in 500 responses
- `src/adapters/primary/dashboard-adapter.ts:208` -- exposes internal filesystem paths in error
- `src/adapters/primary/dashboard-hub.ts:252` -- exposes `err.message` in 500 responses
- `src/adapters/primary/dashboard-hub.ts:383` -- echoes user-controlled `subPath` in error response
- `src/adapters/primary/dashboard-hub.ts:414` -- exposes `err.message` from `registerProject` failures
- `src/adapters/secondary/ruflo-adapter.ts:38-46` -- `SwarmParseError` stores raw CLI output

**Issue:** Multiple error handlers return `err.message` directly to HTTP clients. Error messages from Node.js fs operations, child_process failures, and JSON parse errors frequently contain internal paths, system usernames, and configuration details.

Specific worst case at `dashboard-adapter.ts:208`:
```typescript
this.json(res, 500, { error: 'Dashboard HTML not found. Searched:\n' + candidates.join('\n') });
```
This leaks up to 5 absolute filesystem paths to any HTTP client.

**Remediation:** Return generic error messages to clients. Log detailed errors to stderr only:
```typescript
// BAD
this.json(res, 500, { error: err instanceof Error ? err.message : 'Internal error' });
// GOOD
process.stderr.write(`[dashboard] Internal: ${String(err)}\n`);
this.json(res, 500, { error: 'Internal server error' });
```

---

### VULN-03: Dashboard Hub Project Registration Allows Arbitrary Path Access (HIGH)

**File:** `src/adapters/primary/dashboard-hub.ts:133-164`

```typescript
async registerProject(rootPath: string): Promise<ProjectSlot> {
  const absPath = resolve(rootPath);
  const id = absPath.split('/').pop() ?? 'unknown';
  // ... creates full AppContext for absPath
  const ctx = await this.contextFactory(absPath);
```

**Issue:** The `POST /api/projects/register` endpoint accepts any `rootPath` from an HTTP request body with zero validation. An attacker on the local network (or via SSRF) can:

1. Register `rootPath: "/"` to create an AppContext rooted at the filesystem root
2. Use `/api/<projectId>/tokens/<file>` to read any `.ts` file on disk (the path traversal guard only blocks `..` and leading `/` within the subpath, but the root itself is `/`)
3. The `glob('**/*.ts')` call on the tokens/overview endpoint would enumerate all TypeScript files on the entire filesystem

The project ID is derived from `absPath.split('/').pop()` which for `/` is empty string, creating collision risk.

Additionally, the CORS policy at line 217 accepts any origin starting with `http://localhost` -- this includes `http://localhost.evil.com`, enabling a same-origin bypass from a malicious website.

**Remediation:**
1. Validate `rootPath` against an allowlist or require it to contain a `package.json`
2. Restrict CORS to exact origin matches: `origin === 'http://localhost:' + port`
3. Add authentication or a shared secret for the registration endpoint

---

### VULN-04: CORS Origin Validation Bypass (MEDIUM)

**Files:**
- `src/adapters/primary/dashboard-adapter.ts:135`
- `src/adapters/primary/dashboard-hub.ts:217`

```typescript
if (origin.startsWith('http://localhost') || origin.startsWith('http://127.0.0.1') || !origin) {
  res.setHeader('Access-Control-Allow-Origin', origin || 'http://localhost');
}
```

**Issue:** The `startsWith` check is insufficient. The following malicious origins pass validation:

- `http://localhost.evil.com` -- attacker-controlled domain
- `http://localhost:1234.evil.com` -- attacker-controlled domain
- `http://127.0.0.1.evil.com` -- attacker-controlled domain

Any website served from these domains can make credentialed cross-origin requests to the dashboard and exfiltrate project data (architecture analysis, file contents via token endpoints, filesystem paths).

**Remediation:** Use exact matching with a regex:
```typescript
const ALLOWED_ORIGINS = /^https?:\/\/(localhost|127\.0\.0\.1)(:\d+)?$/;
if (ALLOWED_ORIGINS.test(origin)) {
  res.setHeader('Access-Control-Allow-Origin', origin);
}
```

---

## Needs Manual Review

### REVIEW-01: Worktree Path Injection via Branch Name

**File:** `src/adapters/secondary/worktree-adapter.ts:27-30,66-68`

```typescript
async create(branchName: string): Promise<WorktreePath> {
  const absolutePath = this.worktreePath(branchName);
  await this.git('worktree', 'add', absolutePath, '-b', branchName);
```

```typescript
private worktreePath(branchName: string): string {
  return join(this.worktreeDir, `hex-${branchName}`);
}
```

**Concern:** If `branchName` contains path separators (e.g., `../../tmp/evil`), the worktree could be created outside the intended directory. While `git worktree add` may reject certain characters, the `join()` path is not validated. Additionally, `branchName` flows directly into `git -b` which creates a real branch -- names like `--exec=malicious` could be interpreted as flags (though `execFile` array args mitigate this for most git implementations).

**Risk:** LOW to MEDIUM depending on how `branchName` is sourced.

---

### REVIEW-02: RufloAdapter Parameter Injection

**File:** `src/adapters/secondary/ruflo-adapter.ts:152-163`

```typescript
private async mcpExec(tool: string, params?: Record<string, unknown>): Promise<...> {
  const args = ['mcp', 'exec', '--tool', tool];
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      args.push('--param', `${k}=${String(v)}`);
    }
  }
  const { stdout } = await execFile(CLI_BIN, [CLI_PKG, ...args], { ... });
```

**Concern:** Values are passed as `--param key=value` where `value` is `String(v)`. While `execFile` prevents shell injection, if `v` contains newlines or the `=` character, it could confuse the CLI's argument parser. For example, a task title containing `--tool=evil_command` would be passed as a single `--param` value, but edge cases in the downstream CLI parser are unknown.

**Risk:** LOW -- `execFile` array args are safe from shell injection. Risk depends on `@claude-flow/cli` argument parsing.

---

### REVIEW-03: Unpinned Dependency with `@latest` Tag

**File:** `src/adapters/secondary/ruflo-adapter.ts:49`

```typescript
const CLI_PKG = '@claude-flow/cli@latest';
```

**Concern:** Every `mcpExec` call runs `npx @claude-flow/cli@latest ...` which downloads and executes the latest published version. A supply-chain compromise of the `@claude-flow/cli` npm package would immediately affect all hex users on their next command invocation, with no version pinning or integrity checking.

**File:** `package.json:52`
```json
"@claude-flow/cli": "^3.5.15",
```

The `^` semver range in package.json allows minor/patch updates. The `@latest` tag in the adapter is more dangerous since it bypasses even semver constraints.

**Risk:** MEDIUM -- supply chain attack vector. Pin to an exact version and use `npm audit` in CI.

---

## Positive Security Findings

These patterns are correctly implemented and deserve recognition:

1. **execFile over exec** -- All subprocess adapters (`git-adapter.ts`, `worktree-adapter.ts`, `build-adapter.ts`, `ruflo-adapter.ts`) use `execFile` with argument arrays, preventing shell injection.

2. **Dashboard XSS prevention** -- `index.html` implements `escapeHtml()`, uses `textContent` and `createEl()` DOM helpers throughout. No `innerHTML =` assignments with user data found. The single `innerHTML` read at line 335 is inside the `escapeHtml` utility itself (safe pattern).

3. **Body size limits** -- Both `dashboard-adapter.ts` (1KB) and `dashboard-hub.ts` (2KB) enforce `MAX_BODY_SIZE` on POST request bodies.

4. **No hardcoded secrets** -- API keys are loaded exclusively from `process.env` in `composition-root.ts`. No tokens, keys, or credentials found in any source file.

5. **Path traversal basic guard** -- `safePath()` blocks `..` traversal in lexical paths (though symlinks bypass it, see VULN-01).

6. **Token detail path validation** -- Both `dashboard-adapter.ts:260` and `dashboard-hub.ts:315` reject `..` and absolute paths in the file parameter.

---

## Remediation Priority

| ID | Severity | Effort | Priority |
|----|----------|--------|----------|
| VULN-03 | HIGH | Medium | **P0** -- unauthenticated arbitrary path registration |
| VULN-01 | HIGH | Low | **P1** -- add realpath check to safePath |
| VULN-04 | MEDIUM | Low | **P1** -- fix CORS regex |
| VULN-02 | MEDIUM | Low | **P2** -- sanitize error messages |
| REVIEW-03 | MEDIUM | Low | **P2** -- pin @latest to exact version |
| REVIEW-01 | LOW-MED | Low | **P3** -- validate branch names |
| REVIEW-02 | LOW | Low | **P3** -- document param value constraints |
