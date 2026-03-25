/// Integration tests for ADR-2603240130 — Declarative Swarm Agent Behavior from YAML.
///
/// Verifies that all agent/swarm YAML definitions load correctly and that the
/// runtime wiring (model selection, phases, cardinality) matches expectations
/// without making real inference calls.
///
/// Covers specs S09, S10.

use hex_cli::pipeline::agent_def::AgentDefinition;
use hex_cli::pipeline::swarm_config::{AgentCardinality, SwarmConfig};

// ── S09: Non-phase agents still get YAML model + context ────────────────────

#[test]
fn hex_coder_yaml_loads_with_preferred_model() {
    let def = AgentDefinition::load("hex-coder").expect("hex-coder.yml must be present in assets");
    assert!(
        def.model.preferred.is_some(),
        "hex-coder.yml must define model.preferred"
    );
    assert!(
        !def.model.preferred.as_deref().unwrap_or("").is_empty(),
        "hex-coder model.preferred must not be empty"
    );
}

#[test]
fn adr_reviewer_yaml_loads_with_preferred_model() {
    let def = AgentDefinition::load("adr-reviewer")
        .expect("adr-reviewer.yml must be present in assets");
    assert!(
        def.model.preferred.is_some(),
        "adr-reviewer.yml must define model.preferred"
    );
}

#[test]
fn planner_yaml_loads_with_preferred_model() {
    let def =
        AgentDefinition::load("planner").expect("planner.yml must be present in assets");
    assert!(
        def.model.preferred.is_some(),
        "planner.yml must define model.preferred"
    );
}

// ── S02: hex-coder has TDD workflow phases ───────────────────────────────────

#[test]
fn hex_coder_yaml_has_tdd_phases() {
    let def = AgentDefinition::load("hex-coder").expect("hex-coder.yml must load");
    let workflow = def.workflow.expect("hex-coder must have a workflow section");
    assert!(
        workflow.phases.len() >= 3,
        "hex-coder workflow must have at least 3 phases (pre_validate, red, green, refactor); got {}",
        workflow.phases.len()
    );
    let phase_ids: Vec<&str> = workflow.phases.iter().map(|p| p.id.as_str()).collect();
    assert!(
        phase_ids.contains(&"red"),
        "hex-coder workflow must include a 'red' phase; got {:?}",
        phase_ids
    );
    assert!(
        phase_ids.contains(&"green"),
        "hex-coder workflow must include a 'green' phase; got {:?}",
        phase_ids
    );
}

// ── S06: Cardinality read from dev-pipeline.yml ─────────────────────────────

#[test]
fn dev_pipeline_loads_successfully() {
    let config = SwarmConfig::load_default();
    // SwarmConfig::load_default() never panics — it falls back to defaults
    // Verify the config has agents
    assert!(
        !config.agents.is_empty(),
        "dev-pipeline.yml must define at least one agent"
    );
}

#[test]
fn hex_coder_cardinality_is_per_workplan_step() {
    let config = SwarmConfig::load_default();
    let cardinality = config.cardinality_for_role("hex-coder");
    assert_eq!(
        cardinality,
        AgentCardinality::PerWorkplanStep,
        "hex-coder in dev-pipeline.yml must have cardinality: per_workplan_step"
    );
}

#[test]
fn hex_reviewer_cardinality_is_per_source_file() {
    let config = SwarmConfig::load_default();
    let cardinality = config.cardinality_for_role("hex-reviewer");
    // hex-reviewer has cardinality: per_source_file in dev-pipeline.yml
    assert_eq!(
        cardinality,
        AgentCardinality::PerSourceFile,
        "hex-reviewer in dev-pipeline.yml must have cardinality: per_source_file"
    );
}

// ── S10: All required agent YAMLs are present (no missing assets) ────────────

#[test]
fn all_core_agent_yamls_load() {
    let roles = ["hex-coder", "adr-reviewer", "planner", "swarm-coordinator"];
    for role in roles {
        let def = AgentDefinition::load(role);
        assert!(
            def.is_some(),
            "Agent YAML for '{}' must be present in embedded assets",
            role
        );
    }
}
