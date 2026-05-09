//! Conway-on-hex tick rule (Bays B2/S34).
//!
//! Carter Bays' classic hexagonal Life rule: a dead cell with exactly 2
//! live neighbours is born; a live cell with 3 or 4 live neighbours
//! survives; otherwise dies. Produces gliders and oscillators on a
//! hexagonal grid the way B3/S23 does on a square one.
//!
//! Pure function: `tick(&Grid) -> Grid`. No I/O, no allocation outside
//! the new Grid. Composed by `usecases::game_loop`.

use std::collections::HashMap;

use super::coord::Coord;
use super::grid::Grid;

const BIRTH_COUNT: usize = 2;
const SURVIVE_MIN: usize = 3;
const SURVIVE_MAX: usize = 4;

/// Advance the grid by one generation.
pub fn tick(grid: &Grid) -> Grid {
    // Tally neighbour counts for every cell adjacent to a live one
    // (live cells included via self-as-neighbour-of-neighbour). Dead
    // cells far from any live one are guaranteed to stay dead so we
    // skip them entirely — the sparse Grid stays sparse.
    let mut counts: HashMap<Coord, usize> = HashMap::new();
    for live in grid.alive_cells() {
        for n in live.neighbours() {
            *counts.entry(n).or_insert(0) += 1;
        }
    }

    let mut next: Vec<Coord> = Vec::new();
    for (cell, count) in counts {
        let was_alive = grid.is_alive(cell);
        let becomes_alive = if was_alive {
            (SURVIVE_MIN..=SURVIVE_MAX).contains(&count)
        } else {
            count == BIRTH_COUNT
        };
        if becomes_alive {
            next.push(cell);
        }
    }
    Grid::from_alive(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_stays_empty() {
        let g = Grid::new();
        assert_eq!(tick(&g).alive_count(), 0);
    }

    #[test]
    fn isolated_cell_dies() {
        let g = Grid::from_alive([Coord::new(0, 0)]);
        let next = tick(&g);
        assert!(!next.is_alive(Coord::new(0, 0)), "isolated cell should die (0 neighbours)");
    }

    #[test]
    fn two_adjacent_cells_birth_two_more() {
        // Two adjacent live cells share 2 mutual neighbours, each of which
        // sees exactly 2 live neighbours → born. The two original cells
        // each have only 1 live neighbour → die. Net: shape rotates.
        let g = Grid::from_alive([Coord::new(0, 0), Coord::new(1, 0)]);
        let next = tick(&g);
        assert_eq!(next.alive_count(), 2, "2-cell line should produce 2 births and 2 deaths");
    }

    #[test]
    fn cell_with_3_neighbours_survives() {
        // Cell at origin with 3 live neighbours (not adjacent to each other);
        // origin should remain alive (count=3 in survive range [3,4]).
        let g = Grid::from_alive([
            Coord::new(0, 0),
            Coord::new(1, 0),
            Coord::new(-1, 0),
            Coord::new(0, 1),
        ]);
        let next = tick(&g);
        assert!(next.is_alive(Coord::new(0, 0)),
            "origin with 3 live neighbours should survive");
    }
}
