//! ASCII renderer — flat-top hex laid out in a staggered grid.
//!
//! Even rows shifted left by half a cell so neighbours visually align.
//! Live cell = `●`, dead = `·`. Pads ±2 cells around the bounding box
//! so the live region has a frame.

use crate::domain::{Coord, Grid};
use crate::ports::IDisplayPort;

pub struct AsciiDisplay {
    pub padding: i32,
}

impl Default for AsciiDisplay {
    fn default() -> Self {
        Self { padding: 2 }
    }
}

impl IDisplayPort for AsciiDisplay {
    fn render(&mut self, generation: u64, grid: &Grid) {
        println!("\n=== generation {} · {} alive ===", generation, grid.alive_count());
        let Some((lo, hi)) = grid.bounds() else {
            println!("(empty)");
            return;
        };
        let pad = self.padding;
        for r in (lo.r - pad)..=(hi.r + pad) {
            // Stagger by row so q-axis alignment looks roughly hex.
            let indent = if r.rem_euclid(2) == 0 { 0 } else { 1 };
            print!("{:width$}", "", width = indent as usize);
            for q in (lo.q - pad)..=(hi.q + pad) {
                let alive = grid.is_alive(Coord::new(q, r));
                print!("{} ", if alive { '●' } else { '·' });
            }
            println!();
        }
    }
}
