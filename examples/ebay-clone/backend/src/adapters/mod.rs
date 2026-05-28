// Placeholder for adapters module

// ADR-2026-05-19-0721: Initial adapter design for eBay clone backend
// Implements primary (HTTP/gRPC) and secondary (DB/External service) ports.
// Referenced in docs/specs/ebay-mvp.json specs ebay-spec-023, ebay-spec-024.

pub mod primary;
pub mod secondary;

// hex analyze -p ebay-clone-backend