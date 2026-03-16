# hex-f1 â Product Requirements

## Summary

I want to create a golang backend and an htmx front end that provides F1 race statistics

## Technical Decisions

- **Stack:** Go + TypeScript
- **Structure:** Multi-stack (backend/ + frontend/)
- **Architecture:** Hexagonal (ports & adapters)
- **Scaffolded by:** hex-intf

## Scope

### In Scope

- [ ] Define domain entities and value objects
- [ ] Define port interfaces (contracts)
- [ ] Implement primary adapter(s)
- [ ] Implement secondary adapter(s)
- [ ] Wire composition root
- [ ] Unit tests (London-school mocks)

### Out of Scope

- _TBD â add items as the project evolves_

## Architecture

```
src/
  core/
    domain/          # Pure business logic, zero external deps
    ports/           # Typed interfaces (contracts)
    usecases/        # Application logic composing ports
  adapters/
    primary/         # Driving adapters (CLI, HTTP, browser)
    secondary/       # Driven adapters (DB, API, filesystem)
  composition-root   # Wires adapters to ports
```

## Next Steps

1. Fill in domain entities based on the summary above
2. Define port interfaces for each boundary
3. Implement adapters
4. Run `hex-intf analyze .` to validate architecture
