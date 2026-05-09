//! Integration tests against known Bays B2/S34 hex-Life behaviours.
//!
//! These are end-to-end correctness checks via the public domain API.
//! If any of them fail the rule implementation has drifted.

use hex_life::domain::{tick, Coord, Grid};

/// A single isolated cell has 0 neighbours and dies.
#[test]
fn isolated_cell_dies_in_one_tick() {
    let g = Grid::from_alive([Coord::new(0, 0)]);
    assert_eq!(tick(&g).alive_count(), 0);
}

/// Two adjacent cells: each has 1 live neighbour (each other) → die.
/// Their two shared neighbours each see exactly 2 live cells → born.
/// Result: a different 2-cell pair perpendicular to the original.
#[test]
fn two_adjacent_cells_rotate_not_grow() {
    let pair = [Coord::new(0, 0), Coord::new(1, 0)];
    let g = Grid::from_alive(pair);
    let next = tick(&g);
    assert_eq!(next.alive_count(), 2, "expected 2 alive after tick, got {}", next.alive_count());
    // Original cells should be dead.
    for c in pair {
        assert!(!next.is_alive(c), "{:?} should have died", c);
    }
}

/// A "Y" shape: center cell with 3 spokes. Center has 3 live neighbours
/// → survives. Each spoke has 1 live neighbour → dies. Each pair of
/// spokes shares one common neighbour with 2 live spokes adjacent → born.
#[test]
fn y_pattern_center_survives() {
    let g = Grid::from_alive([
        Coord::new(0, 0),     // center
        Coord::new(1, 0),     // E spoke
        Coord::new(-1, 0),    // W spoke
        Coord::new(0, 1),     // SE spoke
    ]);
    let next = tick(&g);
    assert!(
        next.is_alive(Coord::new(0, 0)),
        "center with 3 live neighbours should survive (got dead)"
    );
}

/// Engineering invariant: tick is deterministic — running twice on the
/// same input must yield identical output.
#[test]
fn tick_is_deterministic() {
    let g = Grid::from_alive([
        Coord::new(0, 0),
        Coord::new(2, -1),
        Coord::new(-1, 1),
        Coord::new(1, 1),
        Coord::new(0, -2),
    ]);
    let a = tick(&g);
    let b = tick(&g);
    let a_cells: std::collections::BTreeSet<_> = a.alive_cells().copied().collect();
    let b_cells: std::collections::BTreeSet<_> = b.alive_cells().copied().collect();
    assert_eq!(a_cells, b_cells);
}

/// Engineering invariant: empty grid is a fixed point.
#[test]
fn empty_grid_fixed_point() {
    let g = Grid::new();
    let next = tick(&g);
    assert_eq!(next.alive_count(), 0);
}

/// Population should not explode for moderate inputs (catches off-by-one
/// neighbour-count bugs that would resurrect everything every tick).
#[test]
fn random_pattern_does_not_explode() {
    // 7-cell cluster at origin
    let g = Grid::from_alive([
        Coord::new(0, 0),
        Coord::new(1, 0),
        Coord::new(-1, 0),
        Coord::new(0, 1),
        Coord::new(0, -1),
        Coord::new(1, -1),
        Coord::new(-1, 1),
    ]);
    let mut current = g;
    for _ in 0..20 {
        current = tick(&current);
        assert!(
            current.alive_count() < 200,
            "explosion detected: {} alive after tick",
            current.alive_count()
        );
    }
}
