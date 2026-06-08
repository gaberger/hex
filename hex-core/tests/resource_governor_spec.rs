//! Behavioral spec for the resource governor's admission decision (ADR-2606080915).
//! This is the independent oracle — the implementation in
//! `hex-core/src/resource_governor.rs` must satisfy it. The agent implementing
//! `admit` does not edit this file.

use hex_core::resource_governor::{admit, parse_mem_available_mb, AdmissionDecision::*};

#[test]
fn fits_with_headroom_is_admitted() {
    // 10 GB model + 2 GB job + 1 GB safety = 13 GB <= 20 GB available.
    assert_eq!(admit(10_000, 2_000, 20_000, 1_000), Admit);
}

#[test]
fn the_oom_case_routes_to_frontier() {
    // Tonight's exact scenario: 29 GB model + 4 GB compile job + 2 GB safety = 35 GB
    // > 30 GB RAM. The governor must route to frontier instead of OOMing locally.
    assert_eq!(admit(29_000, 4_000, 30_000, 2_000), RouteToFrontier);
}

#[test]
fn model_alone_too_big_routes_to_frontier() {
    // 40 GB model can't fit in 30 GB even with no job.
    assert_eq!(admit(40_000, 0, 30_000, 1_000), RouteToFrontier);
}

#[test]
fn exact_boundary_fits() {
    // 18 + 1 + 1 = 20, exactly equal to 20 available → fits (<=, not <).
    assert_eq!(admit(18_000, 1_000, 20_000, 1_000), Admit);
}

#[test]
fn one_over_boundary_routes() {
    // 18 + 1 + 1001 = 20_001 > 20_000 available → does not fit.
    assert_eq!(admit(18_000, 1_000, 20_000, 1_001), RouteToFrontier);
}

#[test]
fn zero_workload_always_admits() {
    assert_eq!(admit(0, 0, 0, 0), Admit);
}

#[test]
fn saturates_without_overflow() {
    // Absurd inputs must not panic; the sum overflows u64 → treat as "does not fit".
    assert_eq!(admit(u64::MAX, u64::MAX, 1_000, u64::MAX), RouteToFrontier);
}

#[test]
fn parses_mem_available_to_mb() {
    let meminfo = "MemTotal:       30000000 kB\nMemFree:         5000000 kB\nMemAvailable:   16384000 kB\nBuffers:          200000 kB\n";
    // 16_384_000 kB / 1024 = 16_000 MB.
    assert_eq!(parse_mem_available_mb(meminfo), Some(16_000));
}

#[test]
fn parses_mem_available_with_varied_whitespace() {
    assert_eq!(parse_mem_available_mb("MemAvailable: 1048576 kB\n"), Some(1_024));
}

#[test]
fn missing_mem_available_is_none() {
    assert_eq!(parse_mem_available_mb("MemTotal: 30000000 kB\nMemFree: 5000000 kB\n"), None);
}

#[test]
fn malformed_mem_available_is_none() {
    assert_eq!(parse_mem_available_mb("MemAvailable:   not_a_number kB\n"), None);
}
