// ADR-2026-05-19-0721: Define core modules for eBay clone backend

pub mod domain;
pub mod ports;
pub mod usecases;
pub mod adapters {
    pub mod primary;
    pub mod secondary;
}