//! IDisplayPort — render a Grid for the operator. Implementation lives
//! in adapters/secondary/. Domain depends on this trait, not the impl.

use crate::domain::Grid;

pub trait IDisplayPort {
    fn render(&mut self, generation: u64, grid: &Grid);
}
