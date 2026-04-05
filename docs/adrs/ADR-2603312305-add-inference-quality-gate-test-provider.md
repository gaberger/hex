# ADR-2603312305: Inference Provider Health Checks and Pruning

**Status:** proposed

## Date
2025-05-25

## Drivers
- Users frequently add inference providers that appear successful but later fail, causing wasted time debugging failed inference calls
- The `hex inference discover` command currently surfaces all discovered providers but cannot automatically filter out non-responsive ones, leading to cluttered outputs
- Lack of immediate feedback during provider configuration means issues may only surface during critical inference tasks
- Need for proactive health management aligns with existing quality gates in ADR-005

## Context
In hexagonal architecture terms, inference providers are secondary adapters that implement the `InferencePort` interface. Currently, when users add providers via `hex inference add` or discover them via `hex inference discover`, no immediate validation occurs. The system assumes connectivity and proper configuration, storing provider configurations in persistence adapters.

This creates a reliability gap: providers may be configured with invalid API keys, incorrect endpoints, network restrictions, or may simply be offline. Users discover these failures only when attempting to use the provider for actual inference tasks, which can interrupt workflows and create frustration.

The existing ADR-005 establishes quality gates for the compile-lint-test feedback loop but doesn't address runtime service dependencies. Inference providers' health and responsiveness represent an API consumption quality gate that should be tested before incorporating them into the system.

From a hexagonal architecture perspective, health checking belongs in the domain as a concept, with a port defining the interface, secondary adapters implementing the actual network calls, and use cases coordinating the validation logic. The prune operation during discovery represents a cleanup use case.

## Decision
We will introduce a health check quality gate for inference providers that validates connectivity and basic functionality before acceptance.

1. We will create a new `ProviderHealthPort` in the ports layer defining a `ping(endpoint: string, apiKey?: string): Promise<PingResult>` method. The domain layer will define `PingResult` with fields for success status, latency, and optional error details.

2. We will modify the `AddInferenceProvider` use case to invoke the health check after gathering provider configuration but before committing to persistence. If the ping returns an empty response or times out, the use case will return a structured error and abort the addition.

3. We will extend the `DiscoverInferenceProviders` use case to accept a `prune: boolean` parameter. When enabled, the use case will test each discovered provider via the `ProviderHealthPort` and filter out non-responsive providers before returning results.

4. We will add a `--prune` flag to the `hex inference discover` CLI command that maps directly to the use case parameter. The CLI adapter will format the results differently when pruning is enabled, showing removed providers in a separate section.

5. Secondary adapters implementing `ProviderHealthPort` will be created for each provider type (OpenAI, Anthropic, etc.), reusing existing configuration and HTTP client infrastructure but implementing a minimal endpoint test (e.g., model listing or smallest possible completion).

## Consequences

### Positive
- Immediate feedback on provider connectivity issues reduces debugging time and user frustration
- Automated filtering during discovery keeps provider lists clean and actionable
- Health check infrastructure can be reused for periodic monitoring and alerting features
- Aligns with the quality gate philosophy established in ADR-005

### Negative
- Adds latency to `hex inference add` and `hex inference discover --prune` commands
- Increases network traffic as each provider is tested upon addition
- May produce false negatives due to temporary network issues or rate limiting
- Adds complexity to provider adapter implementations (must support ping endpoint)

### Neutral
- Existing providers in the system remain unchanged; health checks only apply to new additions
- The `--prune` flag is optional; discovery without pruning behaves identically to current implementation
- Health check failures don't automatically remove existing providers, only prevent new additions

## Implementation

### Phases
1. **Tier 2 (Domain/Ports)**: Define `ProviderHealthPort` interface and `PingResult` domain model. Add `prune` parameter to `DiscoverInferenceProvidersUseCase` signature.
2. **Tier 3 (Secondary Adapters)**: Implement health check adapters for each inference provider type, starting with OpenAI and Anthropic.
3. **Tier 1 (Use Cases)**: Modify `AddInferenceProviderUseCase` to invoke health check and fail on empty/error responses. Update `DiscoverInferenceProvidersUseCase` to filter based on health checks when prune is enabled.
4. **Tier 0 (Primary Adapters)**: Update CLI commands: add health check error handling to `inference add`, add `--prune` flag to `inference discover`. Add corresponding MCP server updates per ADR-019.

### Affected Layers
- [x] domain/ (PingResult model, provider health status concept)
- [x] ports/ (ProviderHealthPort interface)
- [x] adapters/primary/ (CLI and MCP server updates)
- [x] adapters/secondary/ (Health check implementations for each provider)
- [x] usecases/ (AddInferenceProviderUseCase, DiscoverInferenceProvidersUseCase updates)
- [x] composition-root (Wire up health port implementations)

### Migration Notes
The changes are backward compatible:
- Existing provider configurations remain valid and functional
- `hex inference discover` without `--prune` flag behaves identically
- `hex inference add` for truly valid providers succeeds as before
- New error messages guide users when health checks fail
- No data migration required; health checks are runtime-only