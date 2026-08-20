//! hex-life — Conway's Game of Life on a hexagonal grid (B2/S34).
//!
//! Hexagonal architecture (ports & adapters) demo:
//!
//!   src/domain/       pure rules + types, zero external deps
//!   src/ports/        typed contracts (IDisplayPort, IInputPort, Command)
//!   src/usecases/     game_loop composing tick + render + input
//!   src/adapters/     concrete I/O (cli stdin, ascii terminal display)
//!   src/composition_root.rs   the ONLY file that imports from adapters
//!   src/main.rs       wires composition + runs
//!
//! Run with:  cargo run -p hex-life

pub mod adapters;
pub mod composition_root;
pub mod domain;
pub mod ports;
pub mod usecases;
