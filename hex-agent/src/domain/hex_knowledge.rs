//! Hex architecture knowledge for context-efficient agent awareness.
//!
//! Provides tiered knowledge injection:
//! - Tier 0 (~300 tokens): always injected — core rules + tools
//! - Tier 1 (~500 tokens): injected when editing specific hex layers
//! - Tier 2 (~2K tokens): injected on demand for architecture work

/// Tier 0: core hex awareness — always included in system prompt.
pub const HEX_AWARENESS: &str = "\
You operate in a hexagonal architecture (ports & adapters) project managed by `hex`. \
Architecture rules (enforced by `hex analyze`):\n\
- domain/ imports only domain/ (zero external deps)\n\
- ports/ imports only domain/ (typed interfaces)\n\
- adapters/primary/ imports only ports/ (driving: CLI, MCP, HTTP)\n\
- adapters/secondary/ imports only ports/ (driven: FS, Git, LLM, DB)\n\
- adapters NEVER import other adapters (no cross-adapter coupling)\n\
- composition-root wires adapters → ports (single DI point)\n\
- All relative imports use .js extensions (NodeNext resolution)\n\n\
Available hex tools:\n\
- `hex analyze .` — architecture health check (run before committing)\n\
- `hex plan <requirements>` — decompose into adapter-bounded tasks\n\
- `hex summarize <path>` — token-efficient AST summary (prefer over reading full files)\n\
- `hex adr list` / `hex adr search <query>` — find Architecture Decision Records\n\
- `hex secrets status` — check available API keys\n\n\
Workflow: before committing run `hex analyze .`. For new external deps propose an ADR in docs/adrs/.";

/// Tier 1: layer-specific rules — injected when editing files in that layer.
pub mod tier1 {
    pub const DOMAIN_RULES: &str = "\
You are editing domain code (core/domain/). Rules:\n\
- ZERO external imports — only other domain/ files\n\
- Pure business logic: value objects, entities, domain events\n\
- No I/O, no async, no framework dependencies\n\
- Domain types are shared vocabulary — changes ripple to ports and adapters";

    pub const PORT_RULES: &str = "\
You are editing port interfaces (core/ports/). Rules:\n\
- Import only from domain/ (for value types)\n\
- Ports are typed interfaces (traits/interfaces) — contracts between layers\n\
- Ports define WHAT, not HOW — no implementation details\n\
- Adding a new port requires an ADR if it introduces external dependency";

    pub const ADAPTER_RULES: &str = "\
You are editing an adapter (adapters/). Rules:\n\
- Import only from ports/ (implement the port interface)\n\
- NEVER import another adapter — adapters are isolated\n\
- Primary adapters (CLI, MCP, HTTP) DRIVE the application\n\
- Secondary adapters (FS, Git, LLM, DB) are DRIVEN by the application\n\
- Use dependency injection — adapters are wired in composition-root";

    pub const USECASE_RULES: &str = "\
You are editing a use case (core/usecases/). Rules:\n\
- Import from domain/ and ports/ only\n\
- Use cases orchestrate business logic by composing port calls\n\
- No direct adapter imports — inject ports via constructor\n\
- Use cases are the main unit of testability (mock ports in tests)";

    pub const COMPOSITION_ROOT_RULES: &str = "\
You are editing the composition root. Rules:\n\
- This is the ONLY file that imports adapters\n\
- Wire adapters → ports here\n\
- API keys loaded from env vars ONLY here\n\
- This file is the dependency injection point";
}

/// Tier 2: extended context — loaded on demand for architecture work.
pub const ADR_CONVENTIONS: &str = "\
ADR Conventions:\n\
- File: docs/adrs/ADR-NNN-short-title.md\n\
- Status lifecycle: proposed → accepted → [deprecated|superseded|abandoned]\n\
- Required sections: Status, Date, Context, Decision, Consequences\n\
- Create ADR for: new ports, new external dependencies, architectural changes\n\
- Use `hex adr list` to see existing ADRs and avoid conflicts";

/// Detect which tier 1 knowledge to inject based on file path.
pub fn tier1_for_path(path: &str) -> Option<&'static str> {
    if path.contains("core/domain") || path.contains("domain/") {
        Some(tier1::DOMAIN_RULES)
    } else if path.contains("core/ports") || path.contains("ports/") {
        Some(tier1::PORT_RULES)
    } else if path.contains("adapters/primary") || path.contains("adapters/secondary") {
        Some(tier1::ADAPTER_RULES)
    } else if path.contains("core/usecases") || path.contains("usecases/") {
        Some(tier1::USECASE_RULES)
    } else if path.contains("composition-root") || path.contains("composition_root") {
        Some(tier1::COMPOSITION_ROOT_RULES)
    } else {
        None
    }
}

/// Detect if the user's message suggests architecture work (triggers tier 2).
pub fn needs_tier2(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("adr")
        || lower.contains("architecture")
        || lower.contains("boundary")
        || lower.contains("hex analyze")
        || lower.contains("hex plan")
        || lower.contains("new port")
        || lower.contains("new adapter")
        || lower.contains("refactor")
        || lower.contains("migrate")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier0_is_reasonable_size() {
        // ~300 tokens ≈ ~1200 chars
        assert!(
            HEX_AWARENESS.len() < 1500,
            "Tier 0 too large: {} chars",
            HEX_AWARENESS.len()
        );
        assert!(
            HEX_AWARENESS.len() > 500,
            "Tier 0 too small: {} chars",
            HEX_AWARENESS.len()
        );
    }

    #[test]
    fn tier1_detects_domain() {
        assert!(tier1_for_path("src/core/domain/value-objects.ts").is_some());
        assert_eq!(
            tier1_for_path("src/core/domain/foo.rs").unwrap(),
            tier1::DOMAIN_RULES
        );
    }

    #[test]
    fn tier1_detects_ports() {
        assert_eq!(
            tier1_for_path("src/core/ports/secrets.ts").unwrap(),
            tier1::PORT_RULES
        );
    }

    #[test]
    fn tier1_detects_adapters() {
        assert_eq!(
            tier1_for_path("src/adapters/primary/cli.rs").unwrap(),
            tier1::ADAPTER_RULES
        );
        assert_eq!(
            tier1_for_path("src/adapters/secondary/fs.rs").unwrap(),
            tier1::ADAPTER_RULES
        );
    }

    #[test]
    fn tier1_detects_usecases() {
        assert_eq!(
            tier1_for_path("src/core/usecases/scaffold.ts").unwrap(),
            tier1::USECASE_RULES
        );
    }

    #[test]
    fn tier1_detects_composition_root() {
        assert_eq!(
            tier1_for_path("src/composition-root.ts").unwrap(),
            tier1::COMPOSITION_ROOT_RULES
        );
    }

    #[test]
    fn tier1_returns_none_for_unknown() {
        assert!(tier1_for_path("README.md").is_none());
        assert!(tier1_for_path("package.json").is_none());
    }

    #[test]
    fn tier2_triggers_on_architecture_keywords() {
        assert!(needs_tier2("Create an ADR for the new database adapter"));
        assert!(needs_tier2("Run hex analyze on the project"));
        assert!(needs_tier2("We need to refactor the ports layer"));
        assert!(needs_tier2("Add a new port for caching"));
    }

    #[test]
    fn tier2_does_not_trigger_on_simple_tasks() {
        assert!(!needs_tier2("Fix the typo in the README"));
        assert!(!needs_tier2("Add a unit test for the parser"));
        assert!(!needs_tier2("Update the version number"));
    }
}
