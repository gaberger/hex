//! The driven port for "what time is it".
//!
//! Reading the wall clock is a side effect, so it is a port. A test can then
//! hand the application a clock that never moves, and assert on the exact
//! timestamp that reaches the disk.

pub use crate::domain::bookmark::Timestamp as TimestampValue;

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> TimestampValue;
}
