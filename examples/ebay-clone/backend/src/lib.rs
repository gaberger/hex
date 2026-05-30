pub mod core;
pub mod adapters;
pub mod composition_root;

// ADR-2026-05-19-0721: Initial module skeleton for eBay clone backend.
// This crate serves as the binary + lib container for the eBay MVP backend.
// See docs/specs/ebay-mvp.json (linked/copied under examples/ebay-clone/docs/specs)
// for specifications ebay-spec-023 and ebay-spec-024 regarding metadata and module structure.
// No business logic is implemented yet; only Cargo metadata and empty module skeletons are present.
// Subsequent workplan steps will populate these modules with auction, listing, and user domain logic.