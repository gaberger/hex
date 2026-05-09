//! Axial hex coordinates.
//!
//! Two-axis (q, r) coordinate system over a flat-top hex grid. The third
//! cubic coordinate `s = -q-r` is implicit; we keep only q+r for storage
//! and for HashMap keys.
//!
//! Six neighbours of `(q, r)`:
//!   E:  (q+1, r)     W:  (q-1, r)
//!   NE: (q+1, r-1)   SW: (q-1, r+1)
//!   NW: (q,   r-1)   SE: (q,   r+1)

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Coord {
    pub q: i32,
    pub r: i32,
}

impl Coord {
    pub const fn new(q: i32, r: i32) -> Self {
        Self { q, r }
    }

    pub const fn neighbours(self) -> [Coord; 6] {
        [
            Coord::new(self.q + 1, self.r),
            Coord::new(self.q - 1, self.r),
            Coord::new(self.q + 1, self.r - 1),
            Coord::new(self.q - 1, self.r + 1),
            Coord::new(self.q, self.r - 1),
            Coord::new(self.q, self.r + 1),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_neighbours_distinct() {
        let n = Coord::new(0, 0).neighbours();
        let mut sorted: Vec<_> = n.into_iter().collect();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 6, "expected 6 distinct neighbours");
    }

    #[test]
    fn neighbours_are_one_step_away() {
        // Cube distance from origin to each neighbour must be 1.
        let origin = Coord::new(0, 0);
        for n in origin.neighbours() {
            let dq = n.q - origin.q;
            let dr = n.r - origin.r;
            let ds = -(dq + dr);
            let dist = (dq.abs() + dr.abs() + ds.abs()) / 2;
            assert_eq!(dist, 1, "neighbour {:?} not 1 step away", n);
        }
    }
}
