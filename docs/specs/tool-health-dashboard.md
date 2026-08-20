# Tool Health Dashboard View

*status*: proposed  ·  *date*: 2026-05-09

Tool Health Dashboard View

**Route**: `#/tools`  
**Persona owner**: tool-czar (ADR-[PHONE])  
**Visual pattern**: Resources.tsx anomaly table + TeamDashboard live agent grid

---

## User Flow

1. **Operator lands on `#/tools`** — page renders immediately with last-known state from SpacetimeDB `/api/tools/health` (cache-first, 5s refresh).
2. **Top-of-page traffic-light grid** — rows = typed tools (repo_grep, cargo_check, web_search, code_patch, adr_draft, workplan_emit, escalate_to_operator), cols = execs + leads (cto, coo, cpo, ciso, engineering-lead, sre-lead, product-lead). Cells display 24h success rate:
   - **Green** ≥ 95%
   - **Yellow** 80–94%
   - **Red** < 80%
   - **Gray** = no data in rolling window
3. **Side panel: system-dep status** — vertical stack of dep cards (rg binary, cargo, ollama models loaded, API keys present). Each card:
   - Icon + name + last-check timestamp (relative, e.g. "2m ago")
   - Status badge (green=ok, yellow=stale >5min, red=missing)
   - Alert pill if missing (click → copy install command to clipboard)
4. **Recent gaps section** — below the grid, list of open `tool_health_observation` rows where `success=false`, sorted desc by timestamp. Each row:
   - Tool name + persona + relative timestamp
   - Truncated error_msg (hover for full text)
   - Link to associated ADR if `tool-czar` drafted one (e.g. "→ ADR-[PHONE]")
   - "Ack" button (marks as handled, removes from list)
5. **Per-tool drill-down modal** — click any grid cell or tool name → modal overlay:
   - Header: tool name + owner persona + tier
   - **Latency sparkline** — p50/p95/p99 over rolling 24h (canvas-rendered, 100px tall)
   - **Recent failure traces** — last 5 error_msg snippets with timestamp + persona
   - **Last reliability change** — timestamp + diff (e.g. "95.2% → 87.1%")
   - **Close** button (ESC key also works)
6. **Escalation indicator (top-right)** — pill badge showing count of unread `escalate_to_operator(urgency=high)` toasts. Click → clears count + opens modal with list of escalations (persona, reason, timestamp, "Dismiss" button per row).

---

## State Transitions

- **Grid cell hover** → tooltip shows exact success rate (e.g. "repo_grep :: cto: 128/130 = 98.5%")
- **Dep card missing** → red border pulses every 2s
- **New failure in recent gaps** → row fades in from top with yellow highlight for 3s
- **Ack button busy state** → spinner replaces text, button disabled until POST `/api/tools/health/ack` returns
- **Modal open** → page scroll locked, dark overlay (click outside or ESC to close)

---

## Observable Artifacts

- **hex-nexus/assets/src/components/views/ToolHealth.tsx** — SolidJS component (follows TeamDashboard + Resources.tsx patterns)
- **hex-nexus/assets/src/services/rest-client.ts** — add `/api/tools/health`, `/api/tools/health/ack`, `/api/tools/deps-status` endpoints
- **hex-nexus/src/orchestration/tool_health_dashboard.rs** — reducer subscriptions to `tool_health_observation`, `tool_health_aggregate`, computes rolling stats
- **Route registered** in `hex-nexus/assets/src/App.tsx` under `<Route path="/tools" component={ToolHealth} />`

---

## Security Correlation (CISO Concerns)

This dashboard surfaces **two OWASP LLM risks** identified in prior audits:

1. **LLM06: Sensitive Information Disclosure** (`lesson:openrouter-content-filter-blocks-security`)  
   - Tool Health grid shows when CISO/adversarial-red personas hit inference 403s containing `secret`, `credential`, `leak` keywords.
   - **Visible metric**: `web_search :: ciso` success rate drops to red → operator sees pattern → switches CISO to Ollama fallback (ADR-[PHONE] mitigation).
   - **Affordance**: Drill-down modal shows recent failure traces with redacted error text; CISO can verify content-filter blocks vs. genuine API failures.

2. **A02: Supply Chain Vulnerabilities** (`lesson:rg-binary-required`, `lesson:web-search-api-key-rotation`)  
   - System-dep status panel alerts on missing `rg`, expired API keys, stale Ollama models.
   - **Visible metric**: dep card turns red + alert pill → operator clicks → copy-pastable remediation (e.g. `cargo install ripgrep`, `export TAVILY_API_KEY=…`).
   - **Escalation indicator** count includes `tool-czar` escalations when a missing dep blocks ≥3 personas (urgency=high).

**Correlation mechanism**: When a tool's success rate drops AND a system dep is red, the Recent Gaps section auto-links both (e.g. "repo_grep failures correlated with missing rg binary"). This prevents the operator hunting across dashboards — security and tooling stay unified in one view.