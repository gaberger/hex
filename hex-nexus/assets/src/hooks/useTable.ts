/**
 * useTable — Generic SolidJS hook that bridges a SpacetimeDB table to a reactive signal.
 *
 * Listens to onInsert / onUpdate / onDelete callbacks on a SpacetimeDB table
 * handle and maintains a SolidJS signal containing the current row set.
 *
 * Usage:
 *   import { useTable } from "./useTable";
 *   const swarms = useTable(() => conn()?.db.swarm);
 *   // swarms() returns Swarm[] — reactively updated
 *
 * The table accessor is a function so it can react to the connection being
 * established (initially null, then populated after connect).
 */
import {
  createSignal,
  createEffect,
  onCleanup,
  type Accessor,
} from "solid-js";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/**
 * Minimal interface that SpacetimeDB generated table handles satisfy.
 * Each table on `conn.db.<tableName>` exposes these methods.
 */
export interface SpacetimeDBTableHandle<Row> {
  onInsert: (cb: (ctx: any, row: Row) => void) => void;
  onUpdate: (cb: (ctx: any, oldRow: Row, newRow: Row) => void) => void;
  onDelete: (cb: (ctx: any, row: Row) => void) => void;
  /** Iterate over all cached rows. */
  [Symbol.iterator](): IterableIterator<Row>;
}

/**
 * Options for customising useTable behaviour.
 */
export interface UseTableOptions<Row> {
  /** Extract a unique key from a row. Defaults to (row as any).id ?? JSON.stringify(row). */
  getKey?: (row: Row) => string;
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

/**
 * Bridge a SpacetimeDB table to a SolidJS signal.
 *
 * @param tableAccessor  A function returning the table handle (or undefined/null
 *                        while the connection is not yet established).
 * @param options         Optional configuration (key extractor, etc.).
 * @returns               An Accessor<Row[]> that updates reactively.
 */
export function useTable<Row>(
  tableAccessor: Accessor<SpacetimeDBTableHandle<Row> | null | undefined>,
  options?: UseTableOptions<Row>,
): Accessor<Row[]> {
  const getKey = options?.getKey ?? defaultGetKey;

  const [rows, setRows] = createSignal<Row[]>([], { equals: false });

  createEffect(() => {
    const table = tableAccessor();
    if (!table) {
      setRows([]);
      return;
    }

    // Seed with current cached rows
    const initial: Row[] = [];
    for (const row of table) {
      initial.push(row);
    }
    setRows(initial);

    // ------ onInsert ------
    table.onInsert((_ctx: any, row: Row) => {
      const key = getKey(row);
      setRows((prev) => {
        // Deduplicate: if row with same key already exists, replace it
        const exists = prev.some((r) => getKey(r) === key);
        if (exists) return prev.map((r) => (getKey(r) === key ? row : r));
        return [...prev, row];
      });
    });

    // ------ onUpdate ------
    table.onUpdate((_ctx: any, oldRow: Row, newRow: Row) => {
      const oldKey = getKey(oldRow);
      setRows((prev) => prev.map((r) => (getKey(r) === oldKey ? newRow : r)));
    });

    // ------ onDelete ------
    table.onDelete((_ctx: any, row: Row) => {
      const key = getKey(row);
      setRows((prev) => prev.filter((r) => getKey(r) !== key));
    });
  });

  return rows;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function defaultGetKey(row: any): string {
  if (row && typeof row === "object") {
    // Most SpacetimeDB tables have an `id` field
    if ("id" in row) return String(row.id);
    // agent-registry heartbeat table keys on agentId
    if ("agentId" in row) return String(row.agentId);
    // inference-gateway provider table keys on providerId
    if ("providerId" in row) return String(row.providerId);
    // inference request/response
    if ("requestId" in row) return String(row.requestId);
    if ("responseId" in row) return String(row.responseId);
    if ("chunkId" in row) return String(row.chunkId);
    // hexflo memory keys on `key`
    if ("key" in row) return String(row.key);
  }
  return JSON.stringify(row);
}
