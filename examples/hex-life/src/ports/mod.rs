//! Ports — typed contracts between domain/usecases and the outside world.
//! Domain may depend on these traits; nothing concrete leaks through.

pub mod display;
pub mod input;

pub use display::IDisplayPort;
pub use input::{Command, IInputPort};
