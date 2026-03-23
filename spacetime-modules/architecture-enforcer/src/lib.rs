//! Architecture Enforcer SpacetimeDB Module
//!
//! Validates hex boundary rules for multi-agent writes.
//! Stores boundary rules and validates proposed imports against them.
//!
//! Tables:
//!   - `boundary_rule` (public) -- hex architecture import rules
//!   - `write_validation` (public) -- validation results for agent writes

#![allow(clippy::too_many_arguments, clippy::needless_borrows_for_generic_args)]

use spacetimedb::{reducer, table, ReducerContext, Table};

// ─── Boundary Rule (PUBLIC) ─────────────────────────────────────────────────

#[table(name = boundary_rule, public)]
#[derive(Clone, Debug)]
pub struct BoundaryRule {
    /// Unique rule identifier (e.g. "domain-no-ports")
    #[unique]
    pub rule_id: String,
    /// Source layer that this rule applies to (e.g. "domain")
    pub source_layer: String,
    /// Import target that is forbidden from this source (e.g. "ports")
    pub forbidden_import: String,
    /// Severity: "error" or "warning"
    pub severity: String,
}

// ─── Write Validation (PUBLIC) ──────────────────────────────────────────────

#[table(name = write_validation, public)]
#[derive(Clone, Debug)]
pub struct WriteValidation {
    /// Unique validation identifier
    #[unique]
    pub validation_id: String,
    /// Agent that proposed the write
    pub agent_id: String,
    /// File path being written
    pub file_path: String,
    /// JSON array of proposed import paths
    pub proposed_imports: String,
    /// Verdict: "approved" or "rejected"
    pub verdict: String,
    /// JSON array of rule descriptions that were violated
    pub violations: String,
    /// ISO 8601 timestamp when validation occurred
    pub validated_at: String,
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Detect the hex architecture layer from a file path.
pub fn detect_layer(path: &str) -> &str {
    if path.contains("composition-root") || path.contains("composition_root") {
        return "composition-root";
    }
    if path.contains("adapters/primary") || path.contains("adapters\\primary") {
        return "adapters/primary";
    }
    if path.contains("adapters/secondary") || path.contains("adapters\\secondary") {
        return "adapters/secondary";
    }
    if path.contains("usecases/") || path.contains("usecases\\") {
        return "usecases";
    }
    if path.contains("ports/") || path.contains("ports\\") {
        return "ports";
    }
    if path.contains("domain/") || path.contains("domain\\") {
        return "domain";
    }
    "unknown"
}

/// Check if an import from `source_layer` to `import_layer` violates the given rule.
pub fn check_violation(
    source_layer: &str,
    import_layer: &str,
    rule_source: &str,
    rule_forbidden: &str,
) -> bool {
    source_layer == rule_source && import_layer.starts_with(rule_forbidden)
}

/// Seed the 6 default hex boundary rules into the database.
#[reducer]
pub fn seed_default_rules(ctx: &ReducerContext) -> Result<(), String> {
    let rules = vec![
        ("domain-no-ports", "domain", "ports", "error"),
        ("domain-no-adapters", "domain", "adapters", "error"),
        ("ports-no-adapters", "ports", "adapters", "error"),
        ("ports-no-usecases", "ports", "usecases", "error"),
        (
            "primary-no-secondary",
            "adapters/primary",
            "adapters/secondary",
            "error",
        ),
        (
            "secondary-no-primary",
            "adapters/secondary",
            "adapters/primary",
            "error",
        ),
    ];

    for (id, source, forbidden, severity) in rules {
        // Upsert: skip if already exists
        if ctx
            .db
            .boundary_rule()
            .rule_id()
            .find(&id.to_string())
            .is_none()
        {
            ctx.db.boundary_rule().insert(BoundaryRule {
                rule_id: id.to_string(),
                source_layer: source.to_string(),
                forbidden_import: forbidden.to_string(),
                severity: severity.to_string(),
            });
        }
    }

    log::info!("Seeded default hex boundary rules");
    Ok(())
}

/// Validate a proposed write against the boundary rules.
///
/// - `proposed_imports_json` is a JSON array of import path strings.
/// - Checks each import against all boundary rules for the source file's layer.
/// - Writes a `WriteValidation` row with the verdict and any violations.
#[reducer]
pub fn validate_write(
    ctx: &ReducerContext,
    validation_id: String,
    agent_id: String,
    file_path: String,
    proposed_imports_json: String,
    validated_at: String,
) -> Result<(), String> {
    let source_layer = detect_layer(&file_path);

    // Parse proposed imports — simple JSON array parsing
    // Strip brackets and quotes, split by comma
    let imports = parse_json_string_array(&proposed_imports_json);

    let rules: Vec<BoundaryRule> = ctx.db.boundary_rule().iter().collect();

    let mut violations: Vec<String> = Vec::new();

    for import_path in &imports {
        let import_layer = detect_layer(import_path);

        for rule in &rules {
            if check_violation(
                source_layer,
                import_layer,
                &rule.source_layer,
                &rule.forbidden_import,
            ) {
                violations.push(format!(
                    "[{}] {} cannot import {} (rule: {})",
                    rule.severity, source_layer, import_layer, rule.rule_id
                ));
            }
        }
    }

    let verdict = if violations.is_empty() {
        "approved"
    } else {
        "rejected"
    };

    let violations_json = format!(
        "[{}]",
        violations
            .iter()
            .map(|v| format!("\"{}\"", v.replace('"', "\\\"")))
            .collect::<Vec<_>>()
            .join(",")
    );

    // Upsert validation result
    if let Some(existing) = ctx
        .db
        .write_validation()
        .validation_id()
        .find(&validation_id)
    {
        ctx.db
            .write_validation()
            .validation_id()
            .update(WriteValidation {
                agent_id,
                file_path: file_path.clone(),
                proposed_imports: proposed_imports_json,
                verdict: verdict.to_string(),
                violations: violations_json.clone(),
                validated_at,
                ..existing
            });
    } else {
        ctx.db.write_validation().insert(WriteValidation {
            validation_id,
            agent_id,
            file_path: file_path.clone(),
            proposed_imports: proposed_imports_json,
            verdict: verdict.to_string(),
            violations: violations_json.clone(),
            validated_at,
        });
    }

    log::info!(
        "Validated write to '{}' (layer: {}): {} ({} violations)",
        file_path,
        source_layer,
        verdict,
        violations.len()
    );

    Ok(())
}

/// Simple JSON string array parser. Handles: ["a", "b", "c"]
fn parse_json_string_array(json: &str) -> Vec<String> {
    let trimmed = json.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Vec::new();
    }

    // Strip outer brackets
    let inner = trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or(trimmed);

    inner
        .split(',')
        .map(|s| s.trim().trim_matches('"').trim_matches('\'').to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Layer detection ─────────────────────────────────────────────────

    #[test]
    fn detect_domain_layer() {
        assert_eq!(detect_layer("src/core/domain/value-objects.ts"), "domain");
    }

    #[test]
    fn detect_ports_layer() {
        assert_eq!(detect_layer("src/core/ports/analyzer.ts"), "ports");
    }

    #[test]
    fn detect_usecases_layer() {
        assert_eq!(detect_layer("src/core/usecases/summarize.ts"), "usecases");
    }

    #[test]
    fn detect_primary_adapter_layer() {
        assert_eq!(
            detect_layer("src/adapters/primary/cli.ts"),
            "adapters/primary"
        );
    }

    #[test]
    fn detect_secondary_adapter_layer() {
        assert_eq!(
            detect_layer("src/adapters/secondary/fs.ts"),
            "adapters/secondary"
        );
    }

    #[test]
    fn detect_composition_root() {
        assert_eq!(detect_layer("src/composition-root.ts"), "composition-root");
    }

    #[test]
    fn detect_unknown_layer() {
        assert_eq!(detect_layer("README.md"), "unknown");
    }

    // ─── Violation checking ──────────────────────────────────────────────

    #[test]
    fn domain_importing_ports_violates() {
        assert!(check_violation("domain", "ports", "domain", "ports"));
    }

    #[test]
    fn domain_importing_adapters_violates() {
        assert!(check_violation(
            "domain",
            "adapters/primary",
            "domain",
            "adapters"
        ));
    }

    #[test]
    fn ports_importing_domain_does_not_violate() {
        assert!(!check_violation("ports", "domain", "domain", "ports"));
    }

    #[test]
    fn primary_importing_secondary_violates() {
        assert!(check_violation(
            "adapters/primary",
            "adapters/secondary",
            "adapters/primary",
            "adapters/secondary"
        ));
    }

    #[test]
    fn usecases_importing_ports_does_not_violate() {
        // No rule forbids usecases -> ports
        assert!(!check_violation("usecases", "ports", "domain", "ports"));
    }

    // ─── JSON parsing ───────────────────────────────────────────────────

    #[test]
    fn parse_empty_array() {
        assert!(parse_json_string_array("[]").is_empty());
    }

    #[test]
    fn parse_single_element() {
        let result = parse_json_string_array("[\"src/domain/foo.ts\"]");
        assert_eq!(result, vec!["src/domain/foo.ts"]);
    }

    #[test]
    fn parse_multiple_elements() {
        let result = parse_json_string_array("[\"a.ts\", \"b.ts\", \"c.ts\"]");
        assert_eq!(result, vec!["a.ts", "b.ts", "c.ts"]);
    }

    #[test]
    fn parse_empty_string() {
        assert!(parse_json_string_array("").is_empty());
    }
}
