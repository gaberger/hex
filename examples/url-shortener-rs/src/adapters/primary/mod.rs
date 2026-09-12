//! Primary adapters — they drive the application.
//!
//! Rule 4 again, from the other side: this imports `ports/` only. It never
//! names a store, a clock, a lifetime or a code width.

pub mod text_console;

pub use text_console::TextConsole;
