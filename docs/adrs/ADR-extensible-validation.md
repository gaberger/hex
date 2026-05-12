# ADR: Extensible Validation System Migration

**Status:** Accepted

## Date

2026-05-01

## Context

hex-core originally had hardcoded validation logic in `domain/validation.rs` with functions like `is_critical_path()` that checked files against a fixed list of system-critical paths (`/etc/passwd`, `/etc/shadow`, etc.). This approach had several limitations:

1. **Not extensible** — adding new validation rules required modifying the domain module
2. **Tight coupling** — consumers of validation logic were coupled to specific validation implementations
3. **No composition** — couldn't combine multiple validation rules or run them independently
4. **Poor separation of concerns** — validation logic mixed domain rules with application-specific checks

As hex grew to support multiple validation scenarios (critical path checks, content validation, architecture boundary enforcement), we needed a way to:
- Register validation rules dynamically at runtime
- Run multiple validation rules against a single file
- Collect all validation errors (not just the first one)
- Follow hexagonal architecture by defining validation as a port

## Decision

Migrate from hardcoded validation functions to a **trait-based validation system** with the following components:

### 1. ValidationRule trait (domain layer)

```rust
// hex-core/src/validation.rs
pub trait ValidationRule: Send + Sync {
    fn validate(&self, path: &str, content: &str) -> Result<(), String>;
}
```

This trait defines the contract for any validation rule. Each rule validates a path and its content, returning either success or a descriptive error message.

### 2. IValidator port (ports layer)

```rust
// hex-core/src/ports/validator.rs
pub trait IValidator: Send + Sync {
    fn add_rule(&mut self, rule: Box<dyn ValidationRule>);
    fn validate_all(&self, path: &str, content: &str) -> Result<(), Vec<String>>;
}
```

The port defines how validation services should behave: they must support rule registration and validation that collects all errors (not fail-fast).

### 3. Validator adapter (adapters layer)

```rust
// hex-agent/src/adapters/validator.rs
pub struct Validator {
    rules: Vec<Box<dyn ValidationRule>>,
}

impl IValidator for Validator {
    fn add_rule(&mut self, rule: Box<dyn ValidationRule>) {
        self.rules.push(rule);
    }

    fn validate_all(&self, path: &str, content: &str) -> Result<(), Vec<String>> {
        let errors: Vec<String> = self.rules
            .iter()
            .filter_map(|rule| rule.validate(path, content).err())
            .collect();

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}
```

The adapter maintains a collection of validation rules and runs all of them, collecting every error message.

### 4. Migration path: CriticalPathRule wrapper

To support gradual migration, the existing `is_critical_path()` function is wrapped in a `ValidationRule` implementation:

```rust
// hex-core/src/validation.rs
pub use crate::domain::validation::is_critical_path;

pub struct CriticalPathRule;

impl ValidationRule for CriticalPathRule {
    fn validate(&self, path: &str, _content: &str) -> Result<(), String> {
        if is_critical_path(path) {
            Err(format!("Cannot modify critical system file: {}", path))
        } else {
            Ok(())
        }
    }
}
```

This allows **both old and new implementations to coexist during the transition**:
- Old code can still call `is_critical_path(path)` directly
- New code can use `CriticalPathRule` with the `IValidator` system
- No breaking changes to existing consumers

### Usage example

```rust
use hex_core::validation::CriticalPathRule;
use hex_agent::adapters::validator::Validator;

let mut validator = Validator::new();
validator.add_rule(Box::new(CriticalPathRule));
validator.add_rule(Box::new(CustomContentRule));

match validator.validate_all("src/foo.rs", "use std::fs;") {
    Ok(()) => println!("All validations passed"),
    Err(errors) => {
        for error in errors {
            eprintln!("Validation error: {}", error);
        }
    }
}
```

## Consequences

### Positive

1. **Extensibility** — new validation rules can be added without modifying core domain code
2. **Composability** — multiple rules can be combined and executed together
3. **Error aggregation** — `validate_all` collects all errors, not just the first one, giving users complete feedback
4. **Testability** — rules can be tested independently; adapters can inject test-specific rules
5. **Separation of concerns** — domain defines the rule interface, ports define the service contract, adapters implement the orchestration
6. **Backward compatibility** — existing `is_critical_path()` calls continue to work during migration

### Negative

1. **Additional indirection** — simple checks now require trait objects and dynamic dispatch
2. **Migration effort** — all existing direct calls to validation functions need to be refactored to use the trait system
3. **Slightly more boilerplate** — each new validation rule requires a struct + trait implementation

### Migration strategy

1. ✅ Add `ValidationRule` trait to `hex-core/src/validation.rs`
2. ✅ Wrap existing `is_critical_path()` in `CriticalPathRule`
3. ✅ Define `IValidator` port in `hex-core/src/ports/validator.rs`
4. ✅ Implement `Validator` adapter in `hex-agent/src/adapters/validator.rs`
5. ✅ Add comprehensive unit tests covering error collection and rule composition
6. 🔲 Refactor call sites to use `IValidator` instead of direct function calls
7. 🔲 Add documentation to composition root showing how to wire up the validator
8. 🔲 (Optional) Deprecate direct validation function exports once all consumers migrate

## Related decisions

- **ADR-014**: No mock.module — dependency injection pattern (similar approach using traits and dependency injection)
- **ADR-001**: Hexagonal architecture (this validation system follows the ports/adapters pattern)

## References

- `hex-core/src/validation.rs` — ValidationRule trait + CriticalPathRule
- `hex-core/src/ports/validator.rs` — IValidator port
- `hex-agent/src/adapters/validator.rs` — Validator adapter implementation
- `hex-core/src/domain/validation.rs` — Original is_critical_path function