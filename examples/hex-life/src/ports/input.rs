//! IInputPort — operator commands at each tick boundary.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Advance one generation.
    Step,
    /// Advance N generations without rendering between.
    StepN(u32),
    /// Reset to initial pattern.
    Reset,
    /// Quit the loop.
    Quit,
}

pub trait IInputPort {
    /// Block until the operator issues a command, or return None when
    /// the input source is closed (e.g. EOF on stdin).
    fn next_command(&mut self) -> Option<Command>;
}
