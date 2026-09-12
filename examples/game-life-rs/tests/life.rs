//! Integration tests for `game-life-rs`.
//!
//! The package is `game-life-rs`. Cargo turns the dash into an underscore, so
//! the import below is `game_life_rs`.
//!
//! These tests see only the public API, on purpose. They pin the contract, so a
//! future rewrite of the internals must pass the same bar.

use game_life_rs::{Grid, GridError};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Builds a grid of the given size with exactly these cells alive.
fn build(width: usize, height: usize, cells: &[(usize, usize)]) -> Grid {
    let mut g = Grid::new(width, height);
    for &(x, y) in cells {
        g.set(x, y, true);
    }
    g
}

/// Lists every live cell, in row order then column order.
fn cells_of(g: &Grid) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for y in 0..g.height() {
        for x in 0..g.width() {
            if g.get(x, y) {
                out.push((x, y));
            }
        }
    }
    out
}

/// Sorts a cell list into row order then column order.
fn normalise(cells: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut out = cells.to_vec();
    out.sort_by_key(|&(x, y)| (y, x));
    out.dedup();
    out
}

/// Asserts the whole grid, not three cells.
///
/// Builds the expected grid with `set` and compares the two grids. A library
/// that also lights a corner fails here, and the `Debug` output shows both
/// pictures.
fn assert_grid(g: &Grid, expected: &[(usize, usize)]) {
    let want = build(g.width(), g.height(), expected);
    assert_eq!(*g, want);
}

/// Asserts that no cell is alive anywhere.
fn assert_empty(g: &Grid) {
    assert_grid(g, &[]);
    assert!(cells_of(g).is_empty());
}

/// Shifts a pattern by `(dx, dy)`.
fn shift(cells: &[(usize, usize)], dx: usize, dy: usize) -> Vec<(usize, usize)> {
    cells.iter().map(|&(x, y)| (x + dx, y + dy)).collect()
}

// ---------------------------------------------------------------------------
// Group A — the four required tests, hardened
//
// Every test here uses a 7 wide by 5 tall grid. Never square: a square grid
// hides a width/height swap.
// ---------------------------------------------------------------------------

/// A1. A blinker oscillates with period 2.
#[test]
fn a1_blinker_has_period_exactly_two() {
    let horizontal = [(2, 1), (3, 1), (4, 1)];
    let vertical = [(3, 0), (3, 1), (3, 2)];

    let mut g = build(7, 5, &horizontal);
    let gen0 = g.clone();

    for round in 0..5 {
        g.step();
        // This kills an empty `pub fn step(&mut self) {}`. A period test that
        // only steps twice and compares proves the period *divides* 2.
        assert_ne!(g, gen0, "round {round}: generation 1 must differ from 0");
        assert_grid(&g, &vertical);

        g.step();
        assert_eq!(g, gen0, "round {round}: generation 2 must return to 0");
        assert_grid(&g, &horizontal);
    }

    assert_eq!(g.generation(), 10);
    assert_eq!(gen0.generation(), 0, "the clone keeps its own counter");
}

/// A2. A block is a still life, for ten steps, not one.
#[test]
fn a2_block_is_a_still_life_for_ten_steps() {
    let block = [(1, 1), (2, 1), (1, 2), (2, 2)];
    let mut g = build(7, 5, &block);
    let start = g.clone();

    for step_number in 1..=10 {
        g.step();
        // An off-by-one that leaks one cell per generation shows up at step 2
        // or later, so compare after *every* step.
        assert_eq!(g, start, "the block changed at step {step_number}");
        assert_grid(&g, &block);
    }
}

/// A3. A lone cell dies, and stays dead.
#[test]
fn a3_lone_cell_dies_and_stays_dead() {
    // The middle of the grid.
    let mut g = build(7, 5, &[(3, 2)]);
    g.step();
    assert_empty(&g);
    // The second step is the whole point. With a stale back buffer the cell
    // comes back to life here.
    g.step();
    assert_empty(&g);
    g.step();
    assert_empty(&g);

    // The corner, where the cell has only 3 neighbours in memory.
    let mut g = build(7, 5, &[(0, 0)]);
    g.step();
    assert_empty(&g);
    g.step();
    assert_empty(&g);
}

/// A4. A cell with exactly three neighbours is born.
#[test]
fn a4_cell_with_three_neighbours_is_born() {
    // An L shape. The dead cell at (2,2) touches all three, so it is born.
    let mut g = build(7, 5, &[(1, 1), (2, 1), (1, 2)]);
    let block = [(1, 1), (2, 1), (1, 2), (2, 2)];

    g.step();
    assert_grid(&g, &block);

    // One move proves the birth rule and the survival rule together.
    g.step();
    assert_grid(&g, &block);
}

// ---------------------------------------------------------------------------
// Group B — the stale-buffer traps
// ---------------------------------------------------------------------------

/// The glider at generation 0.
const GLIDER: [(usize, usize); 5] = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];
/// The glider at generation 1. Hand-checked.
const GLIDER_GEN1: [(usize, usize); 5] = [(0, 1), (2, 1), (1, 2), (2, 2), (1, 3)];
/// The glider at generation 2. Hand-checked.
const GLIDER_GEN2: [(usize, usize); 5] = [(2, 1), (0, 2), (2, 2), (1, 3), (2, 3)];
/// The glider at generation 3. Hand-checked.
const GLIDER_GEN3: [(usize, usize); 5] = [(1, 1), (2, 2), (3, 2), (1, 3), (2, 3)];

/// B1. The glider. A glider never repeats in place, so a stale buffer can never
/// match it.
#[test]
fn b1_glider_walks_diagonally() {
    let mut g = build(12, 12, &GLIDER);
    let gen0 = g.clone();

    g.step();
    assert_grid(&g, &GLIDER_GEN1);
    assert_ne!(g, gen0, "generation 1 must differ from generation 0");

    g.step();
    assert_grid(&g, &GLIDER_GEN2);
    assert_ne!(g, gen0, "generation 2 must differ from generation 0");

    g.step();
    assert_grid(&g, &GLIDER_GEN3);
    assert_ne!(g, gen0, "generation 3 must differ from generation 0");

    // Four steps move the whole shape by one cell on each axis.
    g.step();
    assert_grid(&g, &shift(&GLIDER, 1, 1));
    assert_eq!(g, build(12, 12, &shift(&GLIDER, 1, 1)));

    // Twenty steps in total move it by five cells on each axis.
    for _ in 0..16 {
        g.step();
    }
    assert_grid(&g, &shift(&GLIDER, 5, 5));
    assert_eq!(g.generation(), 20);
}

/// B2. A pattern that dies out must stay dead.
#[test]
fn b2_dead_pattern_stays_dead() {
    let mut g = build(5, 5, &[(0, 0), (1, 0), (2, 0)]);

    g.step();
    assert_grid(&g, &[(1, 0), (1, 1)]);

    g.step();
    assert_empty(&g);

    // With a stale back buffer, the three original cells come back here.
    g.step();
    assert_empty(&g);
    g.step();
    assert_empty(&g);
}

/// B3. A blinker in a corner does not oscillate. It dies in two generations.
///
/// Anybody who writes "corner blinker, period 2" gets a red test on correct
/// code. These numbers are hand-checked.
#[test]
fn b3_corner_blinker_decays_it_does_not_oscillate() {
    let start = [(0, 0), (1, 0), (2, 0)];
    let mut g = build(5, 5, &start);
    let gen0 = g.clone();

    g.step();
    assert_ne!(g, gen0);
    assert_grid(&g, &[(1, 0), (1, 1)]);

    g.step();
    assert_ne!(g, gen0, "a corner blinker never returns to its start");
    assert_empty(&g);

    g.step();
    assert_empty(&g);
}

// ---------------------------------------------------------------------------
// Group C — geometry and edges
// ---------------------------------------------------------------------------

/// C1. Width and height are not the same thing.
#[test]
fn c1_width_and_height_are_different_axes() {
    let mut g = Grid::new(7, 3);

    assert_eq!(g.width(), 7);
    assert_eq!(g.height(), 3);

    assert!(g.try_set(6, 0, true), "x = 6 is the last column");
    assert!(g.try_set(0, 2, true), "y = 2 is the last row");
    assert!(!g.try_set(7, 0, true), "x = 7 is one past the last column");
    assert!(!g.try_set(0, 3, true), "y = 3 is one past the last row");

    assert_eq!(g.try_get(6, 0), Some(true));
    assert_eq!(g.try_get(0, 2), Some(true));
    assert_eq!(g.try_get(7, 0), None);
    assert_eq!(g.try_get(0, 3), None);

    // Outside is dead, and `get` never panics.
    assert!(!g.get(7, 0));
    assert!(!g.get(0, 3));
    assert!(!g.get(usize::MAX, usize::MAX));

    // The failed writes left nothing behind.
    assert_grid(&g, &[(6, 0), (0, 2)]);
}

/// C2. Nothing wraps at the right edge.
///
/// In a flat buffer the last cell of one row sits next to the first cell of the
/// next row. This test proves they are not neighbours.
#[test]
fn c2_no_wrapping_at_the_right_edge() {
    let mut g = build(5, 5, &[(4, 1), (4, 2), (4, 3)]);

    g.step();
    assert_grid(&g, &[(3, 2), (4, 2)]);

    // If the grid wrapped, (0,2) would see three neighbours and be born.
    for y in 0..5 {
        assert!(!g.get(0, y), "column 0 must stay dead at row {y}");
    }

    g.step();
    assert_empty(&g);
}

/// C3. Every cell alive. Only the four corners survive.
///
/// A corner has 3 neighbours, an edge cell has 5, and an interior cell has 8.
/// One assertion pins all three at once.
#[test]
fn c3_full_grid_leaves_only_the_corners() {
    let mut g = Grid::new(5, 7);
    for y in 0..7 {
        for x in 0..5 {
            g.set(x, y, true);
        }
    }

    g.step();
    assert_grid(&g, &[(0, 0), (4, 0), (0, 6), (4, 6)]);

    // Four lone corners are each alone, so the grid empties.
    g.step();
    assert_empty(&g);
}

/// C4. A `set` past the right edge panics.
#[test]
#[should_panic(expected = "outside")]
fn c4_set_past_the_right_edge_panics() {
    let mut g = Grid::new(7, 5);
    g.set(7, 0, true);
}

/// C4. A `set` past the bottom edge panics.
#[test]
#[should_panic(expected = "outside")]
fn c4_set_past_the_bottom_edge_panics() {
    let mut g = Grid::new(7, 5);
    g.set(0, 5, true);
}

/// C4. The panic message names the coordinate and the grid size.
#[test]
#[should_panic(expected = "set(7, 2) is outside a 5x9 grid")]
fn c4_panic_message_names_the_coordinate_and_the_size() {
    let mut g = Grid::new(5, 9);
    g.set(7, 2, true);
}

// ---------------------------------------------------------------------------
// Group D — thin and empty shapes
// ---------------------------------------------------------------------------

/// D1. Degenerate sizes do not panic.
#[test]
fn d1_degenerate_sizes_do_not_panic() {
    let sizes = [(0, 0), (0, 5), (5, 0), (1, 1), (1, 5), (5, 1), (2, 2)];
    for (w, h) in sizes {
        let mut g = Grid::new(w, h);
        assert_eq!(g.width(), w, "width changed for {w}x{h}");
        assert_eq!(g.height(), h, "height changed for {w}x{h}");

        for _ in 0..3 {
            g.step();
            // Invariant 7: `step` never resizes.
            assert_eq!(g.width(), w, "step resized the width of {w}x{h}");
            assert_eq!(g.height(), h, "step resized the height of {w}x{h}");
        }
        assert_eq!(g.generation(), 3);
        assert_empty(&g);
    }
}

/// D2. A 1-wide grid follows a hand-checked chain.
///
/// In a 1-wide grid a cell has at most 2 neighbours, so nothing can ever be
/// born.
#[test]
fn d2_one_wide_grid_decays_by_hand_checked_steps() {
    let mut g = build(1, 5, &[(0, 0), (0, 1), (0, 2), (0, 3), (0, 4)]);

    g.step();
    assert_grid(&g, &[(0, 1), (0, 2), (0, 3)]);
    g.step();
    assert_grid(&g, &[(0, 2)]);
    g.step();
    assert_empty(&g);
    g.step();
    assert_empty(&g);
}

/// D2, mirrored. The same chain along the other axis.
#[test]
fn d2_one_tall_grid_decays_by_hand_checked_steps() {
    let mut g = build(5, 1, &[(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)]);

    g.step();
    assert_grid(&g, &[(1, 0), (2, 0), (3, 0)]);
    g.step();
    assert_grid(&g, &[(2, 0)]);
    g.step();
    assert_empty(&g);
    g.step();
    assert_empty(&g);
}

/// D3. The empty grid.
#[test]
fn d3_zero_by_zero_grid_is_total() {
    let mut g = Grid::new(0, 0);
    assert_eq!(g.width(), 0);
    assert_eq!(g.height(), 0);
    assert!(!g.get(0, 0));
    assert_eq!(g.try_get(0, 0), None);
    assert!(!g.try_set(0, 0, true));

    g.step();
    assert_eq!(g.generation(), 1);
    assert_eq!(g, Grid::new(0, 0));
}

/// Invariant 8. An empty grid stays empty forever.
#[test]
fn d4_empty_stays_empty() {
    let mut g = Grid::new(9, 4);
    for _ in 0..25 {
        g.step();
        assert_empty(&g);
    }
    assert_eq!(g.generation(), 25);
}

/// `set` writes the front buffer, so the caller sees the write at once, and the
/// write survives the next swap.
#[test]
fn d5_set_writes_the_visible_picture() {
    let mut g = Grid::new(7, 5);
    g.set(3, 2, true);
    assert!(g.get(3, 2), "the write must be visible before any step");

    g.set(3, 2, false);
    assert!(!g.get(3, 2), "setting false must clear the cell");

    // Set a block, step, and the block is still there. A write that landed in
    // the back buffer would disappear at the swap.
    for &(x, y) in &[(1, 1), (2, 1), (1, 2), (2, 2)] {
        g.set(x, y, true);
    }
    g.step();
    assert_grid(&g, &[(1, 1), (2, 1), (1, 2), (2, 2)]);
}

// ---------------------------------------------------------------------------
// Group E — oracles that do not restate the rule
// ---------------------------------------------------------------------------

/// A second, deliberately slow neighbour counter, written a different way.
///
/// It uses `i64` offsets, loops `dy` and `dx` from -1 to 1, skips the centre,
/// and bounds-checks every neighbour by hand. It shares no code with the
/// library.
fn oracle_step(width: usize, height: usize, cells: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let w = width as i64;
    let h = height as i64;
    let live = normalise(cells);

    let alive = |x: i64, y: i64| -> bool {
        if x < 0 || y < 0 || x >= w || y >= h {
            return false;
        }
        // A linear scan. Deliberately slow, and it shares no ordering
        // assumption with `normalise`.
        live.contains(&(x as usize, y as usize))
    };

    let mut out = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let mut n = 0u32;
            for dy in -1..=1i64 {
                for dx in -1..=1i64 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    if alive(x + dx, y + dy) {
                        n += 1;
                    }
                }
            }
            let now = alive(x, y);
            if n == 3 || (n == 2 && now) {
                out.push((x as usize, y as usize));
            }
        }
    }
    out
}

/// Every hand-checked pattern in this file, as `(width, height, before, after)`.
#[allow(clippy::type_complexity)]
fn hand_checked_patterns() -> Vec<(usize, usize, Vec<(usize, usize)>, Vec<(usize, usize)>)> {
    vec![
        // Group A.
        (
            7,
            5,
            vec![(2, 1), (3, 1), (4, 1)],
            vec![(3, 0), (3, 1), (3, 2)],
        ),
        (
            7,
            5,
            vec![(3, 0), (3, 1), (3, 2)],
            vec![(2, 1), (3, 1), (4, 1)],
        ),
        (
            7,
            5,
            vec![(1, 1), (2, 1), (1, 2), (2, 2)],
            vec![(1, 1), (2, 1), (1, 2), (2, 2)],
        ),
        (7, 5, vec![(3, 2)], vec![]),
        (7, 5, vec![(0, 0)], vec![]),
        (
            7,
            5,
            vec![(1, 1), (2, 1), (1, 2)],
            vec![(1, 1), (2, 1), (1, 2), (2, 2)],
        ),
        // Group B: the glider, one generation at a time.
        (12, 12, GLIDER.to_vec(), GLIDER_GEN1.to_vec()),
        (12, 12, GLIDER_GEN1.to_vec(), GLIDER_GEN2.to_vec()),
        (12, 12, GLIDER_GEN2.to_vec(), GLIDER_GEN3.to_vec()),
        (12, 12, GLIDER_GEN3.to_vec(), shift(&GLIDER, 1, 1)),
        // Group B: the corner blinker decays.
        (5, 5, vec![(0, 0), (1, 0), (2, 0)], vec![(1, 0), (1, 1)]),
        (5, 5, vec![(1, 0), (1, 1)], vec![]),
        // Group C: the right edge does not wrap.
        (5, 5, vec![(4, 1), (4, 2), (4, 3)], vec![(3, 2), (4, 2)]),
        (5, 5, vec![(3, 2), (4, 2)], vec![]),
        // Group C: a full grid leaves only the corners.
        (
            5,
            7,
            (0..7).flat_map(|y| (0..5).map(move |x| (x, y))).collect(),
            vec![(0, 0), (4, 0), (0, 6), (4, 6)],
        ),
        (5, 7, vec![(0, 0), (4, 0), (0, 6), (4, 6)], vec![]),
        // Group D: thin grids.
        (
            1,
            5,
            vec![(0, 0), (0, 1), (0, 2), (0, 3), (0, 4)],
            vec![(0, 1), (0, 2), (0, 3)],
        ),
        (1, 5, vec![(0, 1), (0, 2), (0, 3)], vec![(0, 2)]),
        (1, 5, vec![(0, 2)], vec![]),
        (
            5,
            1,
            vec![(0, 0), (1, 0), (2, 0), (3, 0), (4, 0)],
            vec![(1, 0), (2, 0), (3, 0)],
        ),
        (5, 1, vec![(1, 0), (2, 0), (3, 0)], vec![(2, 0)]),
        (5, 1, vec![(2, 0)], vec![]),
    ]
}

/// E1. Pin the oracle against every hand-checked pattern, before any test
/// trusts it as a judge.
///
/// Without this step the oracle is only a second program by the same author
/// from the same hour. When the two disagree, there would be no way to know
/// which side is wrong.
#[test]
fn e1_oracle_agrees_with_every_hand_checked_pattern() {
    for (w, h, before, after) in hand_checked_patterns() {
        assert_eq!(
            normalise(&oracle_step(w, h, &before)),
            normalise(&after),
            "the oracle disagrees with a hand-checked {w}x{h} pattern: {before:?}"
        );
    }
}

/// E1, the other half. The library agrees with the same hand-checked table.
#[test]
fn e1_library_agrees_with_every_hand_checked_pattern() {
    for (w, h, before, after) in hand_checked_patterns() {
        let mut g = build(w, h, &before);
        g.step();
        assert_eq!(
            cells_of(&g),
            normalise(&after),
            "the library disagrees with a hand-checked {w}x{h} pattern: {before:?}"
        );
    }
}

/// A ten-line xorshift64 generator. No dependency, and a failure is
/// reproducible forever.
struct Rng(u64);

impl Rng {
    /// A xorshift seeded with zero stays at zero forever, so the seed is forced
    /// odd. Without this guard every random grid comes out empty, every
    /// comparison passes, and the suite proves nothing while printing green.
    fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }

    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// True about 35% of the time. 89 out of 256 is 34.8%.
    fn coin(&mut self) -> bool {
        (self.next() >> 56) < 89
    }
}

/// Fills a grid at about 35% density from one seed.
fn random_grid(width: usize, height: usize, seed: u64) -> Grid {
    let mut rng = Rng::new(seed);
    let mut g = Grid::new(width, height);
    for y in 0..height {
        for x in 0..width {
            if rng.coin() {
                g.set(x, y, true);
            }
        }
    }
    g
}

/// The seed guard itself. Two seeds must produce two different grids.
#[test]
fn e2_two_seeds_produce_different_grids() {
    let a = random_grid(13, 7, 1);
    let b = random_grid(13, 7, 2);
    assert_ne!(a, b, "seed 1 and seed 2 must differ");
    assert!(!cells_of(&a).is_empty(), "seed 1 must not be empty");
    assert!(!cells_of(&b).is_empty(), "seed 2 must not be empty");
}

/// E2. Random grids against the oracle, after every step.
#[test]
fn e2_random_grids_match_the_oracle() {
    // Thin shapes are where index maths breaks, so they are in the list.
    for &(w, h) in &[(13usize, 7usize), (1, 9), (9, 1), (7, 13), (2, 2)] {
        for seed in 1..=200u64 {
            let mut g = random_grid(w, h, seed);
            let mut expected = cells_of(&g);

            for step_number in 1..=8 {
                g.step();
                expected = oracle_step(w, h, &expected);
                assert_eq!(
                    cells_of(&g),
                    normalise(&expected),
                    "{w}x{h} seed {seed} disagrees at step {step_number}"
                );
            }
        }
    }
}

/// Turns a grid 90 degrees clockwise. A `w x h` grid becomes an `h x w` grid.
fn rotate(g: &Grid) -> Grid {
    let (w, h) = (g.width(), g.height());
    let mut out = Grid::new(h, w);
    for y in 0..h {
        for x in 0..w {
            if g.get(x, y) {
                out.set(h - 1 - y, x, true);
            }
        }
    }
    out
}

/// Mirrors a grid left to right.
fn mirror(g: &Grid) -> Grid {
    let (w, h) = (g.width(), g.height());
    let mut out = Grid::new(w, h);
    for y in 0..h {
        for x in 0..w {
            if g.get(x, y) {
                out.set(w - 1 - x, y, true);
            }
        }
    }
    out
}

/// E3. The rule treats every direction alike, so rotate-then-step must equal
/// step-then-rotate. The shape changes from `w x h` to `h x w`, so this checks
/// the index maths too. This test never restates the rule.
#[test]
fn e3_rotation_commutes_with_step() {
    for seed in 1..=20u64 {
        for &(w, h) in &[(13usize, 7usize), (5, 9), (1, 6), (6, 1), (4, 4)] {
            let g = random_grid(w, h, seed);

            let mut rotate_then_step = rotate(&g);
            rotate_then_step.step();

            let mut stepped = g.clone();
            stepped.step();
            let step_then_rotate = rotate(&stepped);

            assert_eq!(
                rotate_then_step, step_then_rotate,
                "{w}x{h} seed {seed}: rotation must commute with step"
            );
        }
    }
}

/// E4. Mirror-then-step must equal step-then-mirror.
#[test]
fn e4_reflection_commutes_with_step() {
    for seed in 1..=20u64 {
        for &(w, h) in &[(13usize, 7usize), (5, 9), (1, 6), (6, 1), (4, 4)] {
            let g = random_grid(w, h, seed);

            let mut mirror_then_step = mirror(&g);
            mirror_then_step.step();

            let mut stepped = g.clone();
            stepped.step();
            let step_then_mirror = mirror(&stepped);

            assert_eq!(
                mirror_then_step, step_then_mirror,
                "{w}x{h} seed {seed}: reflection must commute with step"
            );
        }
    }
}

/// E5. A pattern far from every edge evolves the same wherever you put it.
#[test]
fn e5_translation_does_not_change_the_future() {
    // An R-pentomino. It grows fast, so the grid is large enough that five
    // steps never touch an edge.
    let pattern = [(1, 0), (2, 0), (0, 1), (1, 1), (1, 2)];
    let (dx, dy) = (2usize, 1usize);

    let mut a = build(30, 24, &shift(&pattern, 8, 8));
    let mut b = build(30, 24, &shift(&pattern, 8 + dx, 8 + dy));

    for step_number in 1..=5 {
        a.step();
        b.step();
        let want = shift(&cells_of(&a), dx, dy);
        assert_eq!(
            cells_of(&b),
            normalise(&want),
            "the shifted pattern diverged at step {step_number}"
        );
    }
}

// ---------------------------------------------------------------------------
// Group F — guards for the future
// ---------------------------------------------------------------------------

/// A 20-line FNV-1a hash over the visible cells.
fn fnv1a(g: &Grid) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for y in 0..g.height() {
        for x in 0..g.width() {
            let byte: u8 = if g.get(x, y) { 1 } else { 0 };
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// F1. A frozen hash.
///
/// Be honest about this test's job. The number came from this same code, so it
/// does not prove day-one correctness. Groups A to E do that. This test guards
/// against *change*: a future rewrite of the internals cannot quietly alter the
/// answer.
#[test]
fn f1_r_pentomino_hash_is_frozen() {
    let pattern = [(1, 0), (2, 0), (0, 1), (1, 1), (1, 2)];
    let mut g = build(31, 17, &shift(&pattern, 14, 7));

    for _ in 0..100 {
        g.step();
    }

    assert_eq!(g.generation(), 100);
    assert_eq!(fnv1a(&g), 0x8c4c_b860_65a2_3dd6);
}

/// F2. `Grid` is `Send` and `Sync`.
///
/// This fails to compile if somebody later adds an `Rc` or a `Cell`.
#[test]
fn f2_grid_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Grid>();
    assert_send_sync::<GridError>();
}

/// F3. A size that cannot fit in memory addresses is reported, not wrapped.
#[test]
#[cfg(target_pointer_width = "64")]
fn f3_size_overflow_is_reported() {
    // The padded multiply overflows.
    assert_eq!(
        Grid::try_new(1 << 32, 1 << 32),
        Err(GridError::SizeOverflow)
    );
    // The padding `+ 2` overflows first.
    assert_eq!(Grid::try_new(usize::MAX, 1), Err(GridError::SizeOverflow));
    assert_eq!(Grid::try_new(1, usize::MAX), Err(GridError::SizeOverflow));
    assert_eq!(
        Grid::try_new(usize::MAX - 1, 1),
        Err(GridError::SizeOverflow)
    );
}

/// F3. `new` panics where `try_new` reports.
#[test]
#[cfg(target_pointer_width = "64")]
#[should_panic(expected = "overflows the address space")]
fn f3_new_panics_on_size_overflow() {
    let _ = Grid::new(1 << 32, 1 << 32);
}

/// The error type prints something a person can read.
#[test]
fn f3_grid_error_displays_and_is_an_error() {
    let e = GridError::SizeOverflow;
    let text = e.to_string();
    assert!(text.contains("memory addresses"), "got {text:?}");
    let _boxed: Box<dyn std::error::Error> = Box::new(e);
}

// ---------------------------------------------------------------------------
// Invariants that need their own test
// ---------------------------------------------------------------------------

/// Invariant 6. `step` is pure: the same grid always gives the same answer.
#[test]
fn i6_step_is_pure() {
    for seed in 1..=10u64 {
        let start = random_grid(11, 6, seed);

        let mut a = start.clone();
        let mut b = start.clone();
        for _ in 0..12 {
            a.step();
            b.step();
        }
        assert_eq!(a, b, "seed {seed}: two runs of the same grid must agree");
    }
}

/// Equality ignores the generation counter and the scratch buffer.
#[test]
fn i_equality_ignores_generation_and_scratch() {
    let mut g = build(7, 5, &[(2, 1), (3, 1), (4, 1)]);
    let gen0 = g.clone();

    g.step();
    g.step();

    assert_eq!(g, gen0, "the same picture must compare equal");
    assert_ne!(
        g.generation(),
        gen0.generation(),
        "the counters differ, and that is fine"
    );
}

/// The hand-written `Debug` prints the picture, not the raw bytes.
#[test]
fn i_debug_prints_a_picture() {
    let g = build(3, 2, &[(0, 0), (2, 1)]);
    let text = format!("{g:?}");
    assert!(text.contains("Grid 3x2 generation 0"), "got {text}");
    assert!(text.contains("#.."), "got {text}");
    assert!(text.contains("..#"), "got {text}");
}

/// A clone is independent. Stepping one grid does not move the other.
#[test]
fn i_clone_is_independent() {
    let mut g = build(12, 12, &GLIDER);
    let copy = g.clone();

    g.step();

    assert_grid(&copy, &GLIDER);
    assert_eq!(copy.generation(), 0);
    assert_eq!(g.generation(), 1);
}
