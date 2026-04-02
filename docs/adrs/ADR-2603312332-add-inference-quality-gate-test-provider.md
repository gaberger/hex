# ADR-2504181120: Inference Provider Quality Gates and Pruning

**Status:** Superseded by ADR-2603312305

## Date
2025-04-18

## Drivers
- User frustration when `hex inference add` adds non-functional providers
- Need for automated cleanup of failing providers
- Maintaining high-quality inference in production systems

## Context
In the current `hex inference` system, providers can be added without validation of their connectivity or functionality. When a provider returns empty responses or becomes unavailable, users only discover this when attempting to use inference capabilities, leading to poor user experience and debugging overhead. This violates the "fail fast" principle and creates operational friction.

Existing ADRs provide context: ADR-005 establishes quality gates in the feedback loop, ADR-013 manages secrets for provider authentication, and ADR-019 requires CLI-MCP parity for all commands. The current provider management system lacks validation at the point of addition and maintenance utilities for removing failing providers.

## Decision
We will implement two quality improvements to the inference provider system:

1. **Provider Validation on Addition**: After `hex inference add`, we will test provider connectivity by making a minimal inference request (e.g., single-token completion) and validate the response. Empty or invalid responses will trigger an error, preventing the addition of non-functional providers.

2. **Provider Discovery with Pruning**: We will extend `hex inference discover` with a `--prune` flag that tests all configured providers and removes those that fail connectivity tests or return empty responses. This provides automated cleanup of degraded providers.

Implementation will follow hexagonal architecture: validation logic belongs in the `domain/` layer (provider health checking), new ports in `ports/` (provider validation interface), adapters in `adapters/secondary/` (concrete validation implementations), and usecases in `usecases/` (CLI command handlers).

## Consequences

### Positive
- Early detection of misconfigured or unavailable providers
- Automated cleanup reduces maintenance burden
- Improved reliability of inference operations
- Consistent with ADR-005 quality gate philosophy

### Negative
- `hex inference add` becomes slower due to validation network requests
- Additional complexity in provider management flows
- False positives possible with rate-limited or expensive providers during testing

### Neutral
- Requires additional error types for provider validation failures
- Validation requests count toward provider usage quotas

## Implementation

### Phases
1. **Domain and Ports (Tier 0-1)**: Add `ProviderHealthCheck` port and `ProviderValidationError` domain types
2. **Primary Adapters (Tier 4)**: Update CLI commands to include validation logic and `--prune` flag handling
3. **Secondary Adapters (Tier 5)**: Implement concrete validation adapters for each provider type
4. **Usecases (Tier 2-3)**: Add `ValidateProvider` and `PruneProviders` usecases

### Affected Layers
- [x] domain/ (Provider, ProviderHealthStatus)
- [x] ports/ (ProviderHealthCheck port)
- [ ] adapters/primary/ (CLI command updates)
- [x] adapters/secondary/ (Provider-specific validation)
- [x] usecases/ (Validation and pruning usecases)
- [ ] composition-root (Provider validation service wiring)

### Migration Notes
Existing providers added without validation will remain in configurations. The `--prune` flag provides migration path. No data migration required as provider configurations are stored as user preferences. Rollback is possible by removing the validation step from `hex inference add` and ignoring the `--prune` flag.