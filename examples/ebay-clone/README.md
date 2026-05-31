# hexBay — eBay-style marketplace (hex example)

A working marketplace built across the full hex stack: a hexagonal **Rust
backend** (axum), a **Solid + Vite + Tailwind frontend**, and a **SpacetimeDB
WASM module**.

## Quick start (demo profile)

```bash
./start.sh
# → http://localhost:4173   (API on http://localhost:8080)
```

`start.sh` builds the backend + frontend and launches them. The backend runs an
**in-memory marketplace pre-seeded with a catalog**, so the app is populated on
first load. No SpacetimeDB needed for the demo.

## What works

| Layer | What it does |
|---|---|
| **Backend** (`backend/`) | Hexagonal axum API: `register`, `listings` (create/list), `bids`, `me/won`. Auctions close by time; winner = highest bidder. Permissive CORS for the browser. Seeds a demo catalog on startup. |
| **Frontend** (`frontend/`) | Solid SPA: product grid with search, condition badges, prices; post-listing, register/login, place-bid, my-won pages. Reads the backend API (`VITE_API`, default `http://localhost:8080`). |
| **Marketplace module** (`spacetime-modules/marketplace/`) | Standalone SpacetimeDB 1.0 module — tables `user/listing/auction/bid/watch` + reducers `register_user/create_listing/place_bid/close_auction/watch_listing`. |

### Acceptance test

```bash
cd backend && cargo test --test acceptance_happy_path
```

Drives the real router in-process: register two users → post a listing → the
second user outbids → the auction closes → the winner sees the won item, plus
the negative paths (self-bid, non-increasing bid, bid-after-close).

## Persistent profile (SpacetimeDB)

The in-memory backend stands in for the marketplace WASM module behind the same
port contracts. To run the persistent module:

```bash
hex stdb publish --modules spacetime-modules --database marketplace
hex stdb call  marketplace register_user alice hash123
hex stdb query --db marketplace "SELECT * FROM listing"
```

> Note: the backend's `stdb_client` adapter (reading listings from STDB) is not
> yet wired — the demo profile uses the in-memory adapter. Connecting the two is
> the remaining step to back the frontend with the persistent catalog.

## Architecture

Hexagonal: `core/domain` → `core/ports` → `core/usecases` → `adapters/{primary,
secondary}` → `composition_root`. The composition root is the only place that
imports adapters; swapping the in-memory adapter for `stdb_client` is a one-file
change. The example depends only on public crates — no hex-internal crates.
