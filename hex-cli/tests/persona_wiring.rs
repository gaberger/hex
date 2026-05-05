//! Static gate for wp-extend-hex-agent-worker-roles P0.0.
//!
//! Hard rule: every `hex agent worker --role <r>` dispatch arm in
//! `hex-cli/src/commands/agent/mod.rs::execute_worker_task` MUST reference the
//! YAML persona — directly via `persona.` / `persona.as_ref` or indirectly via
//! one of the persona-derived shared variables (`persona_model`,
//! `persona_max_iter`, `persona_constraints`, `resolved_model`).
//!
//! Why: the 6 hardcoded worker arms historically used `*Agent::from_env()` with
//! `None, None` for model_override / provider, which silently bypassed the
//! YAML's `model.preferred`. P0.0 closes that loophole. This test fails the
//! build if any new arm is added without persona wiring.
//!
//! How it works: parse the source file once, locate each known arm's body
//! (between its `"<role>" =>` marker and the next arm marker / `_ =>`), and
//! assert the slice contains a persona reference.

const SOURCE: &str = include_str!("../src/commands/agent/mod.rs");

const REQUIRED_ARMS: &[&str] = &[
    "hex-coder",
    "hex-reviewer",
    "hex-tester",
    "hex-documenter",
    "hex-ux",
    "hex-fixer",
];

/// Tokens that count as "this arm is wired to the persona". Any one is
/// sufficient — they all transitively read from the loaded `persona` binding.
const PERSONA_TOKENS: &[&str] = &[
    "persona.",
    "persona.as_ref",
    "persona_model",
    "persona_max_iter",
    "persona_constraints",
    "resolved_model",
];

fn arm_body<'a>(src: &'a str, role: &str, all_roles: &[&str]) -> Option<&'a str> {
    let marker = format!("\"{}\" =>", role);
    let start = src.find(&marker)?;

    // Find the earliest "next arm" or "fallback arm" marker after this one.
    let mut end = src.len();
    for other in all_roles {
        if *other == role {
            continue;
        }
        let m = format!("\"{}\" =>", other);
        if let Some(pos) = src[start + marker.len()..].find(&m) {
            let abs = start + marker.len() + pos;
            if abs < end {
                end = abs;
            }
        }
    }
    // Also stop at the generic fallback `_ =>` (the YAML-driven executor).
    if let Some(pos) = src[start + marker.len()..].find("        _ => {") {
        let abs = start + marker.len() + pos;
        if abs < end {
            end = abs;
        }
    }
    Some(&src[start..end])
}

#[test]
fn every_worker_arm_references_persona() {
    // Sanity: the persona must be loaded before dispatch.
    assert!(
        SOURCE.contains("AgentDefinition::load(role)"),
        "execute_worker_task must call AgentDefinition::load(role) before dispatch \
         (wp-extend-hex-agent-worker-roles P0.0). The shared `persona` binding is \
         the foundation for every arm's persona reference."
    );

    let mut missing: Vec<String> = Vec::new();
    for role in REQUIRED_ARMS {
        let Some(body) = arm_body(SOURCE, role, REQUIRED_ARMS) else {
            missing.push(format!("{role}: dispatch arm not found"));
            continue;
        };
        if !PERSONA_TOKENS.iter().any(|tok| body.contains(tok)) {
            missing.push(format!(
                "{role}: arm body does not reference any of {:?} — \
                 this arm bypasses the YAML persona (P0.0 violation)",
                PERSONA_TOKENS
            ));
        }
    }

    assert!(
        missing.is_empty(),
        "PERSONA BYPASS detected in worker dispatch (wp-extend-hex-agent-worker-roles P0.0):\n{}",
        missing.join("\n")
    );
}

#[test]
fn generic_fallback_requires_persona() {
    // The `_ =>` arm is the YAML-driven executor for unknown roles. It must
    // bail loudly when the persona is missing — anything else would silently
    // run a role with no model/constraints/prompt.
    assert!(
        SOURCE.contains("no YAML persona for role"),
        "Generic dispatch arm must error out with `no YAML persona for role` \
         when the persona is missing (P0.1)."
    );
}
