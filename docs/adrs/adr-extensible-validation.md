# ADR: Extensible Validation

## Status

Proposed

## Context

The current implementation uses a hardcoded `is_critical_path` function to determine if a path is critical. This approach lacks flexibility and makes it difficult to add new validation rules in the future.

## Decision

Introduce a `ValidationRule` trait to allow for extensible and flexible validation logic. During the transition, both the old and new implementations will coexist.

## Consequences

- **Flexibility**: Easier to add new validation rules without modifying existing code.
- **Maintainability**: Codebase becomes more maintainable with clear separation of concerns.
- **Transition Period**: Both old and new implementations will coexist during the migration period.

## Migration Plan

### Old Implementation

```rust
fn is_critical_path(path: &str) -> bool {
    path.contains("/critical/")
}
```

### New Implementation

1. Define the `ValidationRule` trait:

```rust
pub trait ValidationRule {
    fn validate(&self, path: &str) -> bool;
}
```

2. Implement the `CriticalPathRule` struct:

```rust
pub struct CriticalPathRule;

impl ValidationRule for CriticalPathRule {
    fn validate(&self, path: &str) -> bool {
        path.contains("/critical/")
    }
}
```

3. Use both implementations during transition:

```rust
fn main() {
    let path = "/some/critical/path";

    // Old implementation
    if is_critical_path(path) {
        println!("Old: Path is critical");
    } else {
        println!("Old: Path is not critical");
    }

    // New implementation
    let rule = CriticalPathRule;
    if rule.validate(path) {
        println!("New: Path is critical");
    } else {
        println!("New: Path is not critical");
    }
}
```

### Validation Commands

- `test -f docs/adrs/adr-extensible-validation.md`
- `grep -q 'ValidationRule' docs/adrs/adr-extensible-validation.md`
- `grep -q 'migration' docs/adrs/adr-extensible-validation.md`