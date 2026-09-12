//! Where the time comes from.

use crate::domain::Timestamp;

/// The current moment.
///
/// The application never calls a clock function directly. It asks this port,
/// so a test can hold time still and step it by hand.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}
