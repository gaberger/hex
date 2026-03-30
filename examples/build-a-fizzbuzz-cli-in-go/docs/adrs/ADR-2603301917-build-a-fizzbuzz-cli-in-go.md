```markdown
# ADR-2309281200: Build FizzBuzz CLI in Go

## Status
proposed

## Date
2023-09-28

## Drivers
- User need for a simple command-line interface to generate FizzBuzz output.

## Context
FizzBuzz is a common programming exercise that demonstrates understanding of basic control structures. As part of this project, we aim to create a command-line interface (CLI) tool that outputs FizzBuzz results based on user-defined input parameters. The implementation will adhere to the hexagonal architecture principles, ensuring clear separation of concerns among the various components of the application.

Existing ADRs for different components of the project primarily focus on building a REST API for a leaderboard. This new initiative will require a different set of components centered around a CLI user interface while maintaining the overall architecture's integrity. The CLI will serve as the primary interface for user interaction, necessitating careful consideration of how different layers of the application will communicate.

## Decision
We will implement the FizzBuzz CLI in Go, utilizing a hexagonal architecture to organize the codebase effectively. The structure will include a `domain` layer to implement the FizzBuzz logic, a `usecases` layer to handle input processing, and an `adapters/primary` layer to serve the command-line interface. The interaction with the `domain` will be through defined ports, ensuring loose coupling and adherence to the architecture's principles.

The CLI will allow users to define parameters such as the range of numbers to evaluate and the rules for Fizz and Buzz, which will steer the main application logic encapsulated in the `domain` layer. This decision enables scalability for future enhancements, such as additional rules or output formats.

## Consequences

### Positive
- Provides a simple tool for users to engage with the FizzBuzz logic interactively.
- Clear separation of concerns through hexagonal architecture enhances maintainability and scalability.

### Negative
- Limited to console-based interaction, which may restrict usability for non-technical users.
- Potential complexity in handling various command-line arguments and user input validations.

### Neutral
- The decision does not directly relate to previously created ADRs focused on the leaderboard REST API.

## Implementation

### Phases
1. **Phase 1**: Implement the `domain` layer to encapsulate the FizzBuzz logic and the `usecases` layer for input handling (Tier 1-2).
2. **Phase 2**: Create the CLI within the `adapters/primary` layer to expose the FizzBuzz functionality to the command line (Tier 3).

### Affected Layers
- [x] domain/
- [x] ports/
- [x] adapters/primary/
- [ ] adapters/secondary/
- [x] usecases/
- [ ] composition-root

### Migration Notes
None
```