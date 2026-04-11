# ADR-2604021215: SpacetimeDB Direct Query CLI Command

**Status:** Accepted
**Date:** 2026-04-02
**Drivers:** Debugging revealed the `project` table was empty in SpacetimeDB while SQLite had data — there was no easy way to verify this without crafting raw curl commands. A `hex stdb query` command would make this transparent.

## Context

During development, state divergence between SpacetimeDB and SQLite is hard to detect. Developers must craft raw HTTP calls to the SpacetimeDB SQL endpoint (`POST /v1/database/{db}/sql`) to inspect table contents. This slows down debugging and increases reliance on dashboard UI which may itself have bugs.

Current state:
- No CLI command exists to query SpacetimeDB directly
- Developers use `curl` to hit `http://127.0.0.1:3033/v1/database/{db}/sql`
- SpacetimeDB SQL API returns a JSON schema+rows format that requires parsing
- There is no way to list available tables from the CLI

Forces:
- SpacetimeDB SQL API is already well-defined and stable
- hex-cli already knows the SpacetimeDB host/database from env vars
- Debugging registration bugs (like the `project` table being empty) is a recurring pain point

## Decision

We will add a `hex stdb` subcommand to hex-cli with the following operations:

```
hex stdb query <SQL>            # Run a SQL SELECT against the default database ("hex")
hex stdb query <SQL> --db <db>  # Query a specific database
hex stdb tables                 # List all tables in the default database
hex stdb tables --db <db>       # List tables in a specific database
```

**Output format:**
- Default: pretty-printed table (aligned columns, truncated at terminal width)
- `--json`: raw JSON output (schema + rows as returned by SpacetimeDB)
- `--count`: print row count only

**Database resolution order:**
1. `--db` flag
2. `HEX_SPACETIMEDB_DATABASE` env var
3. Default: `"hex"` (the core coordination database)

**Host resolution:**
1. `HEX_SPACETIMEDB_HOST` env var
2. Default: `http://127.0.0.1:3033`

The command delegates directly to SpacetimeDB's HTTP SQL API — no nexus daemon required.

## Consequences

**Positive:**
- Instant table inspection without curl or dashboard
- Makes state divergence between SQLite and SpacetimeDB immediately visible
- Enables quick reducer debugging during development
- No nexus dependency — works even when nexus is down

**Negative:**
- Read-only (SQL API supports SELECT only, not mutations)
- Requires SpacetimeDB to be running

**Mitigations:**
- Clearly document read-only constraint in help text
- Show a friendly error if SpacetimeDB is unreachable

## Implementation

| Phase | Description | Status |
|-------|------------|--------|
| P1 | Add `hex stdb` subcommand skeleton to hex-cli | Pending |
| P2 | Implement `query` subcommand with pretty-print + JSON modes | Pending |
| P3 | Implement `tables` subcommand | Pending |
| P4 | Wire into hex-cli command router | Pending |
| P5 | Add MCP tool `hex_stdb_query` | Pending |

## References

- SpacetimeDB SQL API: `POST /v1/database/{db}/sql`
- ADR-2603231500: hexflo-coordination publishes to "hex" database
- `hex-core/src/lib.rs`: `STDB_DATABASE_CORE`, `STDB_MODULE_DATABASES`
