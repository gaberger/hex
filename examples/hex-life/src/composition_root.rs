//! Composition root — the ONLY file that imports from adapters.
//! All other layers (domain, ports, usecases) reference traits.

use std::io::{self, BufReader, BufWriter, Stdin, Stdout};

use crate::adapters::primary::CliInput;
use crate::adapters::secondary::AsciiDisplay;
use crate::domain::{Coord, Grid};
use crate::usecases::GameLoop;

/// Build a starter pattern: a small "spinner" — three live cells in a
/// straight axial line. Under B2/S34 this oscillates with period 2.
pub fn starter_pattern() -> Grid {
    Grid::from_alive([
        Coord::new(-1, 0),
        Coord::new(0, 0),
        Coord::new(1, 0),
    ])
}

pub struct WiredApp {
    pub game: GameLoop,
    pub input: CliInput<BufReader<Stdin>, BufWriter<Stdout>>,
    pub display: AsciiDisplay,
}

pub fn wire() -> WiredApp {
    WiredApp {
        game: GameLoop::new(starter_pattern()),
        input: CliInput::new(BufReader::new(io::stdin()), BufWriter::new(io::stdout())),
        display: AsciiDisplay::default(),
    }
}
