# hex-fizzbuzz-hermes

Minimal hexagonal-architecture Rust example. Built **end-to-end by the hex autonomous SOP loop** as the Phase 0+ proof for ADR-2026-05-14-1135 (hex-as-hermes-harness roadmap).

## Layers

- `src/lib.rs` — domain (`fizzbuzz` fn), ports (`Writer` trait), usecases (`play` fn), unit tests.
- `src/main.rs` — primary adapter (stdout) + composition root.

## Build & test
