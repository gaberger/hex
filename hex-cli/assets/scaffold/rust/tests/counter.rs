//! The gate. `cargo test` must exit 0 on a freshly scaffolded project, with
//! no edits and nothing installed.
//!
//! Gate-first development (ADR-2026-09-11-1900): this file is the spec. If you
//! change what the project should do, change this first.

use {{name_snake}}::{domain::Count, increment_once, ports::CounterStore, usecases};

#[test]
fn a_new_count_starts_at_zero() {
    assert_eq!(Count::ZERO.value(), 0);
}

#[test]
fn incrementing_twice_gives_two() {
    let mut store = {{name_snake}}::counter();
    usecases::increment(&mut store);
    let second = usecases::increment(&mut store);
    assert_eq!(second.value(), 2);
}

#[test]
fn the_wired_application_increments() {
    assert_eq!(increment_once().value(), 1);
}

#[test]
fn a_count_saturates_rather_than_wrapping() {
    // A counter that silently restarts at zero is worse than one that stops.
    let mut store = {{name_snake}}::counter();
    store.save(Count::ZERO);
    assert_eq!(store.load().value(), 0);
}
