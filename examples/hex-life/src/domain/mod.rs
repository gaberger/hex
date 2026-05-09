//! Pure domain — no external deps, no I/O.

pub mod cell;
pub mod coord;
pub mod grid;
pub mod rules;

pub use cell::Cell;
pub use coord::Coord;
pub use grid::Grid;
pub use rules::tick;
