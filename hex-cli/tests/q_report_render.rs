/// P2.5 — Q-report rendering tests.
///
/// - Golden test: table renderer vs fixture under tests/golden/q-report-default.txt
/// - JSON round-trip: QReportEntry serialise → deserialise identity
/// - Trend symbol boundary: -0.05, 0.0, +0.05

use hex_cli::commands::inference::{format_trend, render_q_report_table, trend_symbol};
use serde_json::json;

// ── fixture data (mirrors nexus /api/inference/q-report response shape) ──

fn fixture_body() -> serde_json::Value {
    json!({
        "ok": true,
        "count": 3,
        "sort": "visits",
        "entries": [
            {
                "action": "qwen3:4b",
                "tier": "t1",
                "task_type": "scaffold",
                "q_value": 0.870,
                "visit_count": 142,
                "last_updated": "2026-04-15T12:00:00Z",
                "trend_7d": 0.050
            },
            {
                "action": "qwen2.5-coder:32b",
                "tier": "t2",
                "task_type": "codegen",
                "q_value": 0.650,
                "visit_count": 87,
                "last_updated": "2026-04-15T08:30:00Z",
                "trend_7d": -0.120
            },
            {
                "action": "devstral-small-2:24b",
                "tier": "t2.5",
                "task_type": "reasoning",
                "q_value": 0.430,
                "visit_count": 23,
                "last_updated": "2026-04-14T20:00:00Z",
                "trend_7d": 0.0
            }
        ]
    })
}

// ── 1. Golden test ──────────────────────────────────────────────────────

#[test]
fn table_renderer_matches_golden() {
    let rendered = render_q_report_table(&fixture_body());

    let golden_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/q-report-default.txt");

    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::write(&golden_path, &rendered).expect("write golden file");
        eprintln!("Golden file updated: {}", golden_path.display());
        return;
    }

    let expected = std::fs::read_to_string(&golden_path)
        .unwrap_or_else(|_| panic!("Golden file missing: {}. Run with UPDATE_GOLDEN=1 to create.", golden_path.display()));

    assert_eq!(
        rendered, expected,
        "\n\nRendered output differs from golden file.\n\
         Run `UPDATE_GOLDEN=1 cargo test -p hex-cli --test q_report_render` to update.\n\n\
         --- actual ---\n{rendered}\n--- expected ---\n{expected}"
    );
}

#[test]
fn table_renderer_empty_entries() {
    let body = json!({ "entries": [] });
    assert_eq!(render_q_report_table(&body), "No q-report entries found.");
}

#[test]
fn table_renderer_missing_entries_key() {
    let body = json!({ "ok": true });
    assert_eq!(render_q_report_table(&body), "No q-report entries found.");
}

#[test]
fn table_renderer_accepts_model_field_alias() {
    let body = json!({
        "entries": [{
            "model": "claude-opus-4-20250514",
            "tier": "t3",
            "task_type": "frontier",
            "visits": 5,
            "q_value": 0.990,
            "trend_7d": 0.010
        }]
    });
    let rendered = render_q_report_table(&body);
    assert!(rendered.contains("claude-opus-4-20250514"), "should use 'model' field");
    assert!(rendered.contains("t3"));
}

// ── 2. JSON round-trip ──────────────────────────────────────────────────

#[test]
fn json_round_trip_q_report_entry() {
    let entry = hex_core::QReportEntry {
        state: "t1_scaffold".into(),
        model: "qwen3:4b".into(),
        q_value: 0.87,
        visits: 42,
        last_seen: "2026-04-15T12:00:00Z".into(),
        trend_7d: Some(0.03),
    };

    let json = serde_json::to_string(&entry).expect("serialize");
    let back: hex_core::QReportEntry = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(entry, back);
}

#[test]
fn json_round_trip_none_trend() {
    let entry = hex_core::QReportEntry {
        state: "t2_codegen".into(),
        model: "qwen2.5-coder:32b".into(),
        q_value: 0.65,
        visits: 3,
        last_seen: "2026-04-15T08:30:00Z".into(),
        trend_7d: None,
    };

    let json = serde_json::to_string(&entry).expect("serialize");
    let back: hex_core::QReportEntry = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(entry, back);
    assert!(json.contains("\"trend_7d\":null"));
}

#[test]
fn json_round_trip_full_body() {
    let body = fixture_body();
    let serialized = serde_json::to_string(&body).expect("serialize");
    let back: serde_json::Value = serde_json::from_str(&serialized).expect("deserialize");
    assert_eq!(body, back);
}

// ── 3. Trend symbol boundary values ─────────────────────────────────────

#[test]
fn trend_symbol_positive() {
    assert_eq!(trend_symbol(0.05), "▲");
    assert_eq!(trend_symbol(0.001), "▲");
    assert_eq!(trend_symbol(1.0), "▲");
}

#[test]
fn trend_symbol_negative() {
    assert_eq!(trend_symbol(-0.05), "▼");
    assert_eq!(trend_symbol(-0.001), "▼");
    assert_eq!(trend_symbol(-1.0), "▼");
}

#[test]
fn trend_symbol_zero() {
    assert_eq!(trend_symbol(0.0), "─");
    assert_eq!(trend_symbol(-0.0), "─");
}

#[test]
fn format_trend_boundaries() {
    assert_eq!(format_trend(Some(0.05)), "+0.050 ▲");
    assert_eq!(format_trend(Some(-0.05)), "-0.050 ▼");
    assert_eq!(format_trend(Some(0.0)), "─");
    assert_eq!(format_trend(None), "─");
}

#[test]
fn format_trend_negative_zero() {
    // IEEE 754 negative zero should render as stable, not degrading.
    assert_eq!(format_trend(Some(-0.0)), "─");
}

#[test]
fn format_trend_large_values() {
    assert_eq!(format_trend(Some(1.234)), "+1.234 ▲");
    assert_eq!(format_trend(Some(-0.1)), "-0.100 ▼");
}
