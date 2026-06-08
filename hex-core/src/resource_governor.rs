//! Resource governor — memory-aware admission control (ADR-2606080915, Tier 0).
//!
//! The OOM that motivated this: a 29 GB model load + a compile-heavy job together
//! exceeded 30 GB RAM and the kernel killed unrelated processes. hex had no memory
//! manager. This module is the pure decision core: given measured memory numbers,
//! decide whether a *local* workload fits — and if not, route it to the frontier
//! fallback (`claude -p`), which needs no local RAM, instead of OOMing.
//!
//! Pure and deterministic on purpose: the caller (a hex-nexus adapter) supplies the
//! measured numbers (MemAvailable, model footprint, etc.); this crate stays dep-free
//! and unit-testable. See `tests/resource_governor_spec.rs` for the behavioral spec.

/// What the governor decides for a candidate local workload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionDecision {
    /// Enough memory with the safety margin — run it locally.
    Admit,
    /// Won't fit locally with headroom — route to the frontier fallback
    /// (`claude -p`), which consumes no local RAM, rather than risk an OOM.
    RouteToFrontier,
}

/// Decide whether a local workload fits in available memory with a safety margin.
///
/// A workload is the *sum* of the model it loads and the concurrent job that uses it
/// (e.g. the compile-heavy `hex do` loop) — co-scheduling those is exactly what OOM'd.
///
/// * `model_footprint_mb` — resident model weights + KV-cache headroom.
/// * `job_headroom_mb` — working-set the concurrent job needs.
/// * `available_mb` — currently available system memory (e.g. `MemAvailable`).
/// * `safety_mb` — extra margin to never consume.
///
/// Returns [`AdmissionDecision::Admit`] iff
/// `model_footprint_mb + job_headroom_mb + safety_mb <= available_mb`,
/// otherwise [`AdmissionDecision::RouteToFrontier`]. Arithmetic must saturate, never
/// overflow/panic, on absurd inputs.
pub fn admit(
    model_footprint_mb: u64,
    job_headroom_mb: u64,
    available_mb: u64,
    safety_mb: u64,
) -> AdmissionDecision {
    let required_mb = model_footprint_mb
        .saturating_add(job_headroom_mb)
        .saturating_add(safety_mb);
    if required_mb <= available_mb {
        AdmissionDecision::Admit
    } else {
        AdmissionDecision::RouteToFrontier
    }
}

/// Parse available system memory (in MB) from the contents of `/proc/meminfo`.
///
/// `/proc/meminfo` has lines like `MemAvailable:   16384000 kB`. Find the
/// `MemAvailable:` line, take its value (always reported in **kB**), and convert to
/// MB (integer division by 1024). Returns `None` if the line is absent or malformed.
///
/// Pure so it's testable without touching the real filesystem; the hex-nexus adapter
/// reads the file and hands the contents here. See `tests/resource_governor_spec.rs`.
pub fn parse_mem_available_mb(meminfo: &str) -> Option<u64> {
    let line = meminfo
        .lines()
        .find(|l| l.trim_start().starts_with("MemAvailable:"))?;
    let kb: u64 = line
        .trim_start()
        .trim_start_matches("MemAvailable:")
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some(kb / 1024)
}
