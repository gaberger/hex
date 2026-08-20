// ADR-2026-05-19-0721: Define core modules for eBay clone backend

pub mod domain;
pub mod ports;
pub mod usecases;

// adapters module is defined in the root of the backend crate, 
// not inside core, per hex-core architectural standards.
// See docs/specs/ebay-mvp.json spec ebay-spec-023 for module boundary details.