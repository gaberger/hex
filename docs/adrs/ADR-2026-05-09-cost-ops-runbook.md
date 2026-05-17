# ADR-2026-05-09: Cost Operations Runbook

## Status

Proposed

## Date

2026-05-09

## Author

COO

## Context

The current workplan `docs/workplans/wp-cost-ops-runbook.json` is orphaned due to an outdated ADR reference. The spec for the cost operations runbook, located at `docs/specs/cost-ops-runbook.md`, needs to be converted into an authoritative ADR to properly link and manage the associated workplan.

## Decision

Convert the existing spec `docs/specs/cost-ops-runbook.md` into an ADR using the `adr_draft` tool. The new ADR will serve as the authoritative document for the cost operations runbook, ensuring alignment with the operational counterpart documented in `docs/specs/cost-and-token-efficiency.md`, owned by the CPO.

## Consequences

1. **HEX_DAILY_COST_LIMIT_USD on-call procedure**: Define the process and responsibilities for handling daily cost limits.
2. **HEX_SOP_COST_GATE_USD auto-hold threshold**: Establish the threshold and mechanism for automatically holding operations when costs exceed a specified limit.
3. **Dashboard alert (not PagerDuty)**: Implement a dashboard-based alerting system instead of relying on PagerDuty, ensuring better integration with our existing monitoring tools.
4. **Cost-breakdown table by persona**: Create a detailed cost breakdown table that categorizes expenses by different personas or roles within the organization.
5. **Kill switches**: Define and document kill switches to quickly halt operations in case of unexpected cost spikes.
6. **Audit cadence**: Establish a regular audit schedule to review and optimize cost management practices.

By converting the spec into an ADR, we ensure that all stakeholders are aligned on the operational procedures for managing costs effectively, improving transparency and accountability within the organization.

## References

- Spec: `docs/specs/cost-ops-runbook.md`
- Workplan (orphaned): `docs/workplans/wp-cost-ops-runbook.json`
- Related document: `docs/specs/cost-and-token-efficiency.md` (owned by CPO)

Once this ADR is finalized and committed, the `workplan_auto_emitter` will automatically link the orphaned workplan to this new ADR.