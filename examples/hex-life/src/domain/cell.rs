//! Cell state for a single hex.

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Cell {
    Dead,
    Alive,
}

impl Cell {
    pub const fn is_alive(self) -> bool {
        matches!(self, Cell::Alive)
    }
}
