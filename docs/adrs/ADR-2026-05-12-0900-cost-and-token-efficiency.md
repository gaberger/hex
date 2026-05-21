# ADR: Cost and Token Efficiency

## Date: 2026-05-12

**Status:** Proposed

## Context

The current system configuration for the SOP executor and twin reviewer involves high token consumption, leading to increased operational costs. The specification document `docs/specs/cost-and-token-efficiency.md` outlines proposed changes to optimize these settings.

## Decision

To address cost inefficiencies, we will implement the following:

1. **SOP Executor Configuration**: Set the maximum tokens (`max_tokens`) to 4096.
2. **Twin Reviewer Configuration**: Adjust the maximum tokens (`TWIN_MAX_TOKENS`) to 512.
3. **Cost Surfaces Draftees**: Introduce tier routing for cost optimization.
4. **Projected Savings**: Achieve projected savings of $400/month through tier pinning, caching, and max_tokens reduction.

## Consequences

- Improved cost management and efficiency.
- Enhanced system performance by optimizing token usage.
- Potential for additional cost-saving measures to be identified during implementation.

## Next Steps

1. **ADR Review**: Submit this ADR for review by the relevant stakeholders.
2. **Workplan Relinking**: Once approved, relink the orphaned workplan `docs/workplans/wp-cost-and-token-efficiency.json` using the new ADR reference.
3. **Implementation**: Proceed with implementing the proposed changes as outlined in the specification document.

## References

- Specification: `docs/specs/cost-and-token-efficiency.md`
- Workplan: `docs/workplans/wp-cost-and-token-efficiency.json`
- Cost Burn Projection: `lesson:cost-burn-projection` (if present in hex memory)

---

**Note**: This ADR will be finalized and implemented after review and approval by the relevant stakeholders.