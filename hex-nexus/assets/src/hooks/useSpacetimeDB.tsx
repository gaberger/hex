/**
 * useSpacetimeDB — SolidJS context provider + hook for SpacetimeDB connection lifecycle.
 *
 * Manages a single DbConnection to one SpacetimeDB module, handling:
 * - WebSocket connection via DbConnection.builder()
 * - Auth token persistence in localStorage
 * - Auto-reconnect with exponential backoff
 * - Subscription to core tables on connect
 *
 * Usage:
 *   <SpacetimeDBProvider module="hexflo-coordination" uri="ws://localhost:3000">
 *     <App />
 *   </SpacetimeDBProvider>
 *
 *   const { conn, connected } = useSpacetimeDB();
 */
import {
  createContext,
  useContext,
  createSignal,
  onCleanup,
  type ParentComponent,
  type Accessor,
} from "solid-js";
import type { DbConnectionImpl } from "spacetimedb";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/** Configuration for the SpacetimeDB provider. */
export interface SpacetimeDBConfig {
  /** WebSocket URI, e.g. "ws://localhost:3000" */
  uri: string;
  /** SpacetimeDB module name, e.g. "hexflo-coordination" */
  module: string;
  /** localStorage key used to persist the auth token */
  tokenKey?: string;
  /** SQL queries to subscribe to immediately after connecting */
  subscribeQueries?: string[];
  /** Factory that builds a typed DbConnection from the generated bindings */
  connectionBuilder: ConnectionBuilderFactory;
}

/**
 * A factory function that accepts (uri, module, token, callbacks) and returns
 * a promise resolving to the connected DbConnection. Each generated module
 * exports its own DbConnection.builder(), so callers wire that in here.
 */
export type ConnectionBuilderFactory = (opts: {
  uri: string;
  module: string;
  token?: string;
  onConnect: (conn: any, token: string) => void;
  onDisconnect: () => void;
  onConnectError: (err: Error) => void;
}) => any; // Returns the DbConnection (type varies per module)

/** The shape exposed by the context. */
export interface SpacetimeDBContextValue {
  /** The live DbConnection instance (null until connected). */
  conn: Accessor<any | null>;
  /** Whether we are currently connected. */
  connected: Accessor<boolean>;
  /** Whether a connection attempt is in progress. */
  connecting: Accessor<boolean>;
  /** The last connection error, if any. */
  error: Accessor<Error | null>;
  /** Manually trigger a reconnect. */
  reconnect: () => void;
}

// ---------------------------------------------------------------------------
// Context
// ---------------------------------------------------------------------------

const SpacetimeDBContext = createContext<SpacetimeDBContextValue>();

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

export const SpacetimeDBProvider: ParentComponent<SpacetimeDBConfig> = (props) => {
  const tokenKey = () => props.tokenKey ?? `stdb_token_${props.module}`;

  const [conn, setConn] = createSignal<any | null>(null);
  const [connected, setConnected] = createSignal(false);
  const [connecting, setConnecting] = createSignal(false);
  const [error, setError] = createSignal<Error | null>(null);

  let retryCount = 0;
  let retryTimer: ReturnType<typeof setTimeout> | undefined;
  let disposed = false;

  // -----------------------------------------------------------------------
  // Connect
  // -----------------------------------------------------------------------
  function connect() {
    if (disposed) return;

    setConnecting(true);
    setError(null);

    const savedToken = localStorage.getItem(tokenKey()) ?? undefined;

    try {
      props.connectionBuilder({
        uri: props.uri,
        module: props.module,
        token: savedToken,
        onConnect: (connection: any, token: string) => {
          if (disposed) return;

          // Persist token for future sessions
          localStorage.setItem(tokenKey(), token);

          setConn(connection);
          setConnected(true);
          setConnecting(false);
          retryCount = 0;

          // Subscribe to requested tables
          if (props.subscribeQueries && props.subscribeQueries.length > 0) {
            try {
              connection
                .subscriptionBuilder()
                .onApplied(() => {
                  // Subscription applied — table data is now available
                })
                .onError((_ctx: any, err: Error) => {
                  console.error("[SpacetimeDB] subscription error:", err);
                })
                .subscribe(props.subscribeQueries);
            } catch (subErr) {
              console.error("[SpacetimeDB] failed to subscribe:", subErr);
            }
          }
        },
        onDisconnect: () => {
          if (disposed) return;
          setConnected(false);
          setConn(null);
          scheduleReconnect();
        },
        onConnectError: (err: Error) => {
          if (disposed) return;
          setError(err);
          setConnecting(false);
          scheduleReconnect();
        },
      });
    } catch (err) {
      setError(err instanceof Error ? err : new Error(String(err)));
      setConnecting(false);
      scheduleReconnect();
    }
  }

  // -----------------------------------------------------------------------
  // Exponential backoff
  // -----------------------------------------------------------------------
  function scheduleReconnect() {
    if (disposed) return;
    const delay = Math.min(1000 * Math.pow(2, retryCount), 30_000);
    retryCount++;
    console.log(`[SpacetimeDB] reconnecting in ${delay}ms (attempt ${retryCount})`);
    retryTimer = setTimeout(() => connect(), delay);
  }

  function reconnect() {
    if (retryTimer) clearTimeout(retryTimer);
    retryCount = 0;
    connect();
  }

  // -----------------------------------------------------------------------
  // Lifecycle
  // -----------------------------------------------------------------------
  connect();

  onCleanup(() => {
    disposed = true;
    if (retryTimer) clearTimeout(retryTimer);
    const c = conn();
    if (c && typeof c.disconnect === "function") {
      try {
        c.disconnect();
      } catch {
        // ignore
      }
    }
  });

  const value: SpacetimeDBContextValue = {
    conn,
    connected,
    connecting,
    error,
    reconnect,
  };

  return (
    <SpacetimeDBContext.Provider value={value}>
      {props.children}
    </SpacetimeDBContext.Provider>
  );
};

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

/**
 * Access the SpacetimeDB connection from any descendant of SpacetimeDBProvider.
 * Throws if used outside the provider tree.
 */
export function useSpacetimeDB(): SpacetimeDBContextValue {
  const ctx = useContext(SpacetimeDBContext);
  if (!ctx) {
    throw new Error("useSpacetimeDB must be used within a <SpacetimeDBProvider>");
  }
  return ctx;
}
