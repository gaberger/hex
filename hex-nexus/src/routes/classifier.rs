//! GET /api/classifier/rules — union of all rule tables for dashboard listing.
//!
//! Projects every static rule table accessible from hex-nexus into a common
//! `{ table, label, signals }` shape so the dashboard can render a single
//! flat list without knowing the internal struct layout of each classifier.

use axum::Json;
use serde::Serialize;
use serde_json::{json, Value};

use hex_core::domain::brain::INTENT_RULES;
use hex_core::quantization::GGUF_RULES;
use hex_core::rules::boundary::LAYER_RULES;

use crate::task_type_classifier::TASK_TYPE_RULES;

#[derive(Debug, Serialize)]
struct RuleEntry {
    table: &'static str,
    label: &'static str,
    signals: Vec<&'static str>,
}

/// GET /api/classifier/rules
pub async fn list_classifier_rules() -> Json<Value> {
    let mut rules: Vec<RuleEntry> = Vec::new();

    // hex-nexus: task-type inference routing (ADR-2604142000)
    for r in TASK_TYPE_RULES {
        rules.push(RuleEntry {
            table: "task_type",
            label: r.label,
            signals: r.signals.to_vec(),
        });
    }

    // hex-nexus: steer directive classifier (ADR-2604131500)
    for r in super::steer::STEER_RULES {
        rules.push(RuleEntry {
            table: "steer",
            label: r.label,
            signals: vec![r.signals],
        });
    }

    // hex-core: sched intent classifier
    for r in INTENT_RULES {
        rules.push(RuleEntry {
            table: "intent",
            label: r.label,
            signals: r.signals.to_vec(),
        });
    }

    // hex-core: GGUF quantization level detection
    for r in GGUF_RULES {
        rules.push(RuleEntry {
            table: "gguf",
            label: r.label,
            signals: r.signals.to_vec(),
        });
    }

    // hex-core: hexagonal layer classification
    for r in LAYER_RULES {
        rules.push(RuleEntry {
            table: "layer",
            label: r.label,
            signals: r.signals.to_vec(),
        });
    }

    Json(json!({
        "ok": true,
        "count": rules.len(),
        "rules": rules,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn union_covers_all_tables() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let Json(val) = rt.block_on(list_classifier_rules());
        let rules = val["rules"].as_array().unwrap();
        let tables: std::collections::HashSet<&str> = rules
            .iter()
            .map(|r| r["table"].as_str().unwrap())
            .collect();
        assert!(tables.contains("task_type"));
        assert!(tables.contains("steer"));
        assert!(tables.contains("intent"));
        assert!(tables.contains("gguf"));
        assert!(tables.contains("layer"));
        assert_eq!(tables.len(), 5);
    }

    #[test]
    fn every_rule_has_label_and_signals() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let Json(val) = rt.block_on(list_classifier_rules());
        for rule in val["rules"].as_array().unwrap() {
            assert!(!rule["label"].as_str().unwrap().is_empty());
            assert!(!rule["signals"].as_array().unwrap().is_empty());
        }
    }
}
