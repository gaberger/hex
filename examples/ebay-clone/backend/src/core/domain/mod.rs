# examples/ebay-clone/backend/src/core/domain/mod.rs — STUB (operator triage required)

**Status:** stub — auto-generated after 2 drafter attempts
**Generated:** 2026-05-28T13:53:36.335091038+00:00
**Committed by:** `hex-coder`
**Original committed path:** `examples/ebay-clone/backend/src/core/domain/mod.rs`
**Commitment:** code_patch: create examples/ebay-clone/backend/src/core/domain/mod.rs

## Why this is a stub

The persona `hex-coder` committed to producing this artifact, but on 2 drafter attempts the drafter could not produce a usable draft. Causes include: persona returned `INSUFFICIENT_CONTEXT`, persona returned an empty draft, content was too short for the long-form artifact type (e.g. ADR / spec), or the artifact path contained unresolved template placeholders like `<auto-id>` that the persona forgot to substitute.

## Originating ask

```
(no thread linkage — DM had no thread_id)
```

## What to do

One of:

1. **Fill it in by hand** — edit this file with the actual content you want for `examples/ebay-clone/backend/src/core/domain/mod.rs`.
2. **Delete this stub** — the commitment is already marked abandoned in STDB so nothing will retry.
3. **Re-ask with more context** — DM `@hex-coder` with a more specific prompt (and an explicit concrete artifact path/ID if the prior failure was a placeholder) and let the responder + drafter pipeline try again. Consider pinning `HEX_DRAFTER_MODEL_LONGFORM` to a stronger model for ADR/spec asks.

---

*Stub written directly by the drafter circuit-breaker. Bypassed twin review because stubs are an operator-triage signal, not a persona artifact. Commitment_id `4229` was abandoned with the abandon reason pointing here. See `hex-nexus/src/orchestration/drafter.rs`.*

---

// BEGIN ADDED CONTENT

pub mod ids;
pub mod money;
pub mod username;
pub mod title;
pub mod time;

use serde::{Serialize, Deserialize};
use std::convert::TryFrom;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum DomainError {
    InvalidUsername,
    InvalidListingTitle,
    InvalidMoneyAmount,
}

// UserId and related types moved to ids.rs
// ListingId, BidId, AuctionId also in ids.rs

// Money type with validation in money.rs

// Username type with validation in username.rs

// ListingTitle type with validation in title.rs

// Timestamp and DurationMs types in time.rs

// END ADDED CONTENT

docs/specs/ebay-spec-023
docs/specs/ebay-spec-024