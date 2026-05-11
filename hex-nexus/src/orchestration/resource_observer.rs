//! Resource observer (ADR-2026-05-08-2200).
//!
//! Walks `/proc` every 15 s on Linux, computes per-process RSS / CPU% /
//! argv signature, and upserts a row into `process_observation` for each
//! interesting process. The 60 s `resource_supervisor_tick` reducer
//! (in hexflo-coordination) consumes these rows to detect duplicate
//! argvs, oversize RSS, zombies, and CPU pin — and writes
//! `resource_anomaly` rows the operator can see in the dashboard.
//!
//! Linux-only by design. macOS / Windows nodes log a warn-once and exit
//! without doing anything.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};

const TICK_ENV: &str = "HEX_RESOURCE_OBSERVER_INTERVAL";
const ALLOW_ENV: &str = "HEX_RESOURCE_OBSERVER_ALLOW";
const DEFAULT_TICK_SECS: u64 = 15;
/// Comm prefixes that are interesting by default. Operators can override
/// via `HEX_RESOURCE_OBSERVER_ALLOW=hex-,ollama,spacetimedb-`.
const DEFAULT_ALLOW: &[&str] = &[
    "hex",
    "ollama",
    "spacetimedb-",
    "claude",
];

/// Per-PID CPU snapshot used to compute deltas between ticks.
#[derive(Clone, Copy, Default)]
struct CpuSnapshot {
    /// utime+stime in jiffies.
    cumulative_jiffies: u64,
    /// SystemTime when we observed.
    sampled_at_secs: u64,
}

pub fn spawn(stdb_host: String, hex_db: String) {
    if !cfg!(target_os = "linux") {
        tracing::warn!("resource_observer: non-Linux host, observer disabled");
        return;
    }

    let interval = std::env::var(TICK_ENV)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(DEFAULT_TICK_SECS);

    let allow_prefixes: Vec<String> = std::env::var(ALLOW_ENV)
        .ok()
        .map(|v| v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect())
        .unwrap_or_else(|| DEFAULT_ALLOW.iter().map(|s| s.to_string()).collect());

    tokio::spawn(async move {
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "resource_observer: failed to build http client; disabled");
                return;
            }
        };
        let url = format!("{}/v1/database/{}/call/process_observation_upsert", stdb_host, hex_db);
        let host = hostname();

        tracing::info!(
            interval_secs = interval,
            allow = ?allow_prefixes,
            url = %url,
            "resource_observer: started"
        );

        // Wait a beat so STDB hydration is done before we hammer it.
        tokio::time::sleep(Duration::from_secs(20)).await;

        let mut cpu_snapshots: HashMap<u32, CpuSnapshot> = HashMap::new();
        let clk_tck = clk_tck() as f64;
        let allow: Arc<Vec<String>> = Arc::new(allow_prefixes);
        let mut ticker = tokio::time::interval(Duration::from_secs(interval));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        // Back-off state for when STDB is unreachable — quiet log spam while
        // it recovers. After 5 consecutive transport failures we skip ticks
        // until ping returns. Resets on first success.
        let stdb_ping_url = format!("{}/v1/ping", stdb_host);
        let mut consecutive_failures: u32 = 0;
        let mut backoff_logged = false;

        loop {
            ticker.tick().await;

            if consecutive_failures >= 5 {
                match http.get(&stdb_ping_url).send().await {
                    Ok(r) if r.status().is_success() => {
                        tracing::info!("resource_observer: STDB recovered, resuming");
                        consecutive_failures = 0;
                        backoff_logged = false;
                    }
                    _ => {
                        if !backoff_logged {
                            tracing::warn!(
                                "resource_observer: STDB unreachable, backing off (silent retries every {}s)",
                                interval
                            );
                            backoff_logged = true;
                        }
                        continue;
                    }
                }
            }

            let entries = match scan_proc(&allow) {
                Ok(e) => e,
                Err(e) => {
                    tracing::debug!(error = %e, "resource_observer: scan failed");
                    continue;
                }
            };

            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);

            let mut next_snapshots: HashMap<u32, CpuSnapshot> = HashMap::new();
            let mut upserted = 0usize;

            for entry in entries {
                let prev = cpu_snapshots.get(&entry.pid).copied();
                let cpu_pct = match prev {
                    Some(p) if entry.cumulative_jiffies >= p.cumulative_jiffies && now_secs > p.sampled_at_secs => {
                        let dj = (entry.cumulative_jiffies - p.cumulative_jiffies) as f64;
                        let dt = (now_secs - p.sampled_at_secs) as f64;
                        if dt > 0.0 {
                            (dj / clk_tck) / dt * 100.0
                        } else {
                            0.0
                        }
                    }
                    _ => 0.0,
                };
                next_snapshots.insert(entry.pid, CpuSnapshot {
                    cumulative_jiffies: entry.cumulative_jiffies,
                    sampled_at_secs: now_secs,
                });

                let body = serde_json::json!([
                    entry.pid,
                    host.clone(),
                    entry.argv_sha,
                    entry.argv_first,
                    entry.state,
                    entry.ppid,
                    entry.started_micros,
                    entry.rss_kb,
                    cpu_pct as f32,
                ]);

                match http.post(&url).json(&body).send().await {
                    Ok(r) if r.status().is_success() => {
                        upserted += 1;
                        consecutive_failures = 0;
                        backoff_logged = false;
                    }
                    Ok(r) => tracing::debug!(
                        status = %r.status(),
                        pid = entry.pid,
                        "resource_observer: upsert non-2xx"
                    ),
                    Err(e) => {
                        consecutive_failures = consecutive_failures.saturating_add(1);
                        tracing::debug!(
                            error = %e,
                            pid = entry.pid,
                            "resource_observer: upsert transport error"
                        );
                    }
                }
            }

            cpu_snapshots = next_snapshots;
            tracing::debug!(upserted, "resource_observer: tick complete");
        }
    });
}

#[derive(Debug)]
struct ProcEntry {
    pid: u32,
    ppid: u32,
    state: String,
    rss_kb: u64,
    cumulative_jiffies: u64,
    started_micros: i64,
    argv_first: String,
    argv_sha: String,
}

fn scan_proc(allow: &[String]) -> std::io::Result<Vec<ProcEntry>> {
    let mut out = Vec::new();
    let dir = std::fs::read_dir("/proc")?;
    let boot_secs = read_btime().unwrap_or(0);
    let clk_tck_u = clk_tck();

    for entry in dir.flatten() {
        let name = entry.file_name();
        let s = match name.to_str() {
            Some(s) => s,
            None => continue,
        };
        if !s.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let pid: u32 = match s.parse() {
            Ok(n) => n,
            Err(_) => continue,
        };

        // /proc/<pid>/stat — gives state, ppid, utime, stime, starttime
        let stat = match std::fs::read_to_string(format!("/proc/{}/stat", pid)) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let (state, ppid, utime, stime, starttime) = match parse_stat(&stat) {
            Some(t) => t,
            None => continue,
        };

        // /proc/<pid>/cmdline — NUL-separated argv
        let argv_raw = std::fs::read(format!("/proc/{}/cmdline", pid)).unwrap_or_default();
        if argv_raw.is_empty() {
            continue; // kernel thread; skip
        }
        let argv_str = argv_raw
            .split(|b| *b == 0u8)
            .filter(|s| !s.is_empty())
            .map(|b| String::from_utf8_lossy(b).into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let comm = argv_str
            .split_whitespace()
            .next()
            .map(|p| p.rsplit('/').next().unwrap_or(p).to_string())
            .unwrap_or_default();

        if !allow.iter().any(|prefix| comm.starts_with(prefix)) {
            continue;
        }

        // /proc/<pid>/status — VmRSS:
        let status = std::fs::read_to_string(format!("/proc/{}/status", pid)).unwrap_or_default();
        let rss_kb: u64 = status
            .lines()
            .find(|l| l.starts_with("VmRSS:"))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or(0);

        let mut hasher = Sha256::new();
        hasher.update(argv_str.as_bytes());
        let argv_sha = hex_lower(&hasher.finalize());
        let mut argv_first = argv_str.clone();
        if argv_first.len() > 240 {
            argv_first.truncate(240);
        }

        // starttime is jiffies since boot; convert to Unix micros.
        let started_micros: i64 = if boot_secs > 0 && clk_tck_u > 0 {
            let secs_since_boot = (starttime as f64) / (clk_tck_u as f64);
            ((boot_secs as f64 + secs_since_boot) * 1_000_000.0) as i64
        } else {
            0
        };

        out.push(ProcEntry {
            pid,
            ppid,
            state,
            rss_kb,
            cumulative_jiffies: utime + stime,
            started_micros,
            argv_first,
            argv_sha,
        });
    }
    Ok(out)
}

/// Parse fields (state, ppid, utime, stime, starttime) from /proc/<pid>/stat.
/// The third field is `state` and may sit AFTER a parenthesised comm that
/// can itself contain spaces or close-parens, so we slice to the LAST `)`.
fn parse_stat(s: &str) -> Option<(String, u32, u64, u64, u64)> {
    let close = s.rfind(')')?;
    let tail = s[close + 1..].trim();
    let mut fields = tail.split_whitespace();
    let state = fields.next()?.to_string();
    let ppid: u32 = fields.next()?.parse().ok()?;
    // After ppid: pgrp, session, tty_nr, tpgid, flags, minflt, cminflt,
    // majflt, cmajflt, utime, stime, cutime, cstime, priority, nice,
    // num_threads, itrealvalue, starttime
    let _ = fields.next()?; // pgrp
    let _ = fields.next()?; // session
    let _ = fields.next()?; // tty_nr
    let _ = fields.next()?; // tpgid
    let _ = fields.next()?; // flags
    let _ = fields.next()?; // minflt
    let _ = fields.next()?; // cminflt
    let _ = fields.next()?; // majflt
    let _ = fields.next()?; // cmajflt
    let utime: u64 = fields.next()?.parse().ok()?;
    let stime: u64 = fields.next()?.parse().ok()?;
    let _ = fields.next()?; // cutime
    let _ = fields.next()?; // cstime
    let _ = fields.next()?; // priority
    let _ = fields.next()?; // nice
    let _ = fields.next()?; // num_threads
    let _ = fields.next()?; // itrealvalue
    let starttime: u64 = fields.next()?.parse().ok()?;
    Some((state, ppid, utime, stime, starttime))
}

fn read_btime() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/stat").ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("btime ") {
            return rest.trim().parse().ok();
        }
    }
    None
}

fn clk_tck() -> u64 {
    // Linux _SC_CLK_TCK is almost always 100. We avoid `libc` dep here.
    100
}

fn hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn hex_lower(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}
