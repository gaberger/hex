

# ADR-230720T1430: Temperature Converter CLI Implementation

## Status
proposed

## Date
2023-07-20

## Drivers
- User requirement for a temperature conversion CLI tool
- Hexagonal architecture enforcement requirements
- Existing hex framework adoption (ADR-001)

## Context
The project requires implementing a temperature conversion CLI tool in Rust. This must adhere to the established hexagonal architecture (ADR-001) with strict layer boundaries. The tool must convert between Celsius, Fahrenheit, and Kelvin. Key constraints include:
- Domain layer must contain pure business logic
- Ports layer must define interfaces for external interactions
- Adapters layer must contain implementation-specific code
- Usecases layer must contain application-specific logic
- No layer may import another layer (ADR-001 compliance)

## Decision
We will implement the temperature converter CLI using the hexagonal architecture pattern. The solution will consist of:
1. **Domain Layer**: Contains temperature conversion algorithms and value objects
2. **Ports Layer**: Defines CLI interaction interfaces (ports)
3. **Adapters Layer**: Implements CLI-specific code (primary adapter)
4. **Usecases Layer**: Orchestrates domain logic with CLI input/output

The implementation will follow these dependencies:
- Adapters layer will depend on Usecases layer
- Usecases layer will depend on Domain layer
- Ports layer will depend on Usecases layer
- All other dependencies prohibited by hex boundary rules

## Consequences

### Positive
- Clear separation of concerns between business logic and CLI implementation
- Easy to add new temperature units or CLI interfaces in future
- Improved testability through dependency injection
- Compliance with existing architectural standards

### Negative
- Initial setup complexity for new developers
- Requires additional infrastructure for CLI input/output handling
- Potential performance overhead from interface abstractions

### Neutral
- No immediate impact on existing system components
- No changes to database or persistence layers

## Implementation

### Phases
1. **Phase 1**: Implement domain layer with temperature conversion algorithms (Tier 0)
2. **Phase 2**: Implement usecases layer to orchestrate conversions (Tier 1)
3. **Phase 3**: Implement primary adapter (CLI) to handle user input/output (Tier 2)

### Affected Layers
- [ ] domain/
- [ ] ports/
- [ ] adapters/primary/
- [ ] adapters/secondary/
- [ ] usecases/
- [ ] composition-root

### Migration Notes
None (new implementation)