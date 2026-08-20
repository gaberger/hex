//! Sparse hex grid — only live cells stored.
//!
//! Storing only live cells keeps the working set small for typical Life
//! patterns (a beacon is 6 cells, a glider is 5). Bounded scan windows
//! during tick come from the union of live cells + their neighbours.

use std::collections::BTreeSet;

use super::coord::Coord;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Grid {
    live: BTreeSet<Coord>,
}

impl Grid {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_alive(cells: impl IntoIterator<Item = Coord>) -> Self {
        Self { live: cells.into_iter().collect() }
    }

    pub fn is_alive(&self, c: Coord) -> bool {
        self.live.contains(&c)
    }

    pub fn alive_count(&self) -> usize {
        self.live.len()
    }

    pub fn alive_cells(&self) -> impl Iterator<Item = &Coord> {
        self.live.iter()
    }

    /// Bounding box (inclusive) of all live cells. None when empty.
    pub fn bounds(&self) -> Option<(Coord, Coord)> {
        let mut iter = self.live.iter().copied();
        let first = iter.next()?;
        let mut min_q = first.q;
        let mut max_q = first.q;
        let mut min_r = first.r;
        let mut max_r = first.r;
        for c in iter {
            if c.q < min_q { min_q = c.q; }
            if c.q > max_q { max_q = c.q; }
            if c.r < min_r { min_r = c.r; }
            if c.r > max_r { max_r = c.r; }
        }
        Some((Coord::new(min_q, min_r), Coord::new(max_q, max_r)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_grid_has_no_bounds() {
        assert!(Grid::new().bounds().is_none());
    }

    #[test]
    fn bounds_of_three_cells() {
        let g = Grid::from_alive([
            Coord::new(0, 0),
            Coord::new(2, -1),
            Coord::new(-1, 3),
        ]);
        let (lo, hi) = g.bounds().unwrap();
        assert_eq!(lo, Coord::new(-1, -1));
        assert_eq!(hi, Coord::new(2, 3));
    }
}
