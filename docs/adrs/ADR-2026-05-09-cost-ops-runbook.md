# ADR-2026-05-09-cost-ops-runbook

## Title: HEX Cost Operations Runbook

**Status:** Accepted

> Status flipped 2026-05-23 (operator). Workplan derivation will land via `workplan_auto_emitter` on next tick.

## Date: 2026-05-09

## Owner: COO

## Context

The current cost management system requires a comprehensive runbook to ensure that the organization stays within budget and operates efficiently. The existing spec at `docs/specs/cost-ops-runbook.md` outlines the necessary procedures and thresholds for HEX's cost operations.

## Decision

Convert the spec at `docs/specs/cost-ops-runbook.md` into an authoritative ADR using the `adr_draft` typed tool. This will provide a standardized document that can be referenced by the team and used to guide operational decisions related to cost management.

## Consequences

1. **HEX_DAILY_COST_LIMIT_USD On-Call Procedure**: Define the process for responding to daily cost limits being exceeded, ensuring timely interventions to prevent further overspending.
2. **HEX_SOP_COST_GATE_USD Auto-Hold Threshold**: Establish an automated threshold that triggers a hold on spending when costs exceed predefined limits, preventing unnecessary expenditures.
3. **Dashboard Alert (Not PagerDuty)**: Implement alerts on the dashboard to notify stakeholders of cost-related issues without relying on PagerDuty for notifications.
4. **Cost-Breakdown Table by Persona**: Create a detailed table that breaks down costs by persona, providing transparency and enabling better resource allocation.
5. **Kill Switches**: Define kill switches that can be activated in case of critical cost issues to immediately halt non-essential spending.
6. **Audit Cadence**: Establish a regular audit cadence to review cost management practices and ensure compliance with organizational policies.

## References

- `docs/specs/cost-ops-runbook.md`
- `docs/workplans/wp-cost-ops-runbook.json`
- `docs/specs/cost-and-token-efficiency.md`

## Next Steps

1. Convert the spec at `docs/specs/cost-ops-runbook.md` into an ADR using the `adr_draft` tool.
2. Once the ADR is finalized, update the workplan at `docs/workplans/wp-cost-ops-runbook.json` to link it with the new ADR.
3. Ensure that all stakeholders are aware of the updated runbook and its implications for cost management operations.

---

This document has been converted from the original spec to ensure consistency and authority across the organization's cost management procedures.