/**
 * connection.ts — Singleton SpacetimeDB connection store for hex-nexus dashboard.
 *
 * Creates a DbConnection to the hexflo-coordination module, subscribes to
 * all core tables, and exports typed SolidJS signal accessors for each table.
 *
 * All reactive primitives (signals + useTable effects) are created inside
 * initConnectionStore() which must be called from App.tsx before any other
 * store initialization (ADR-2603231000).
 *
 * Usage:
 *   import { swarms, tasks, agents, connected } from "../stores/connection";
 *   // In a component:  <For each={swarms()}>{(s) => ...}</For>
 */

// Suppress SpacetimeDB SDK's internal "send was called before connect" errors
// These occur when the SDK tries to send logs during disconnect - safe to ignore
if (typeof window !== 'undefined') {
  const originalError = console.error;
  console.error = (...args: any[]) => {
    const msg = args[0]?.toString() || '';
    if (msg.includes('send was called before connect')) {
      return; // Suppress this specific SDK internal error
    }
    originalError.apply(console, args);
  };
}
import {
  createSignal,
  createRoot,
  type Accessor,
} from "solid-js";
import { useTable, type SpacetimeDBTableHandle } from "../hooks/useTable";
import {
  DbConnection as HexfloDbConnection,
} from "../spacetimedb/hexflo-coordination/index";
// agent-registry module no longer used — ADR-058 moved agents to hexflo-coordination (hex_agent table)
import {
  DbConnection as InferenceGatewayDbConnection,
} from "../spacetimedb/inference-gateway/index";
// ADR-2604050900: fleet-state module deleted; compute_node absorbed into hexflo-coordination

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/** Resolve SpacetimeDB URI: localStorage override > window.location fallback. */
function resolveSpacetimeDbUri(): string {
  const override = localStorage.getItem("stdb_uri_override");
  if (override) {
    console.log(`[stdb] Using localStorage override: ${override}`);
    return override;
  }

  // Use window.location to construct WebSocket URL dynamically
  const proto = window.location.protocol === "https:" ? "wss:" : "ws:";
  const host = window.location.hostname;
  const port = 3033;
  const uri = `${proto}//${host}:${port}`;
  console.log(`[stdb] Connecting to SpacetimeDB at ${uri}`);
  return uri;
}

const SPACETIMEDB_URI = resolveSpacetimeDbUri();
const TOKEN_KEY_PREFIX = "stdb_token_";

// ---------------------------------------------------------------------------
// Row shape exports (consumed by derived memos — e.g. inbox unread counts)
// ---------------------------------------------------------------------------

/**
 * Runtime shape of an `agent_inbox` row as delivered by the SpacetimeDB client.
 * Field names are snake_case to match the wire column names used by existing
 * consumers (InboxPanel, App.tsx memos). Timestamps are ISO strings; an empty
 * or null `acknowledged_at` indicates the notification is unread.
 */
export interface AgentInboxRow {
  id: number | bigint;
  agent_id: string;
  priority: number;
  kind: string;
  payload: string;
  created_at: string;
  acknowledged_at: string | null;
  expired_at: string | null;
}

// ---------------------------------------------------------------------------
// Connection state signals — assigned inside createRoot by initConnectionStore
// ---------------------------------------------------------------------------

// hexflo-coordination
let hexfloConn: Accessor<any | null> = () => null;
let setHexfloConn: (v: any | null) => void = () => {};
let hexfloConnected: Accessor<boolean> = () => false;
let setHexfloConnected: (v: boolean) => void = () => {};

// agent-registry — retired (ADR-058), kept as stubs for backwards compat
let agentRegistryConnected: Accessor<boolean> = () => false;

// inference-gateway
let inferenceConn: Accessor<any | null> = () => null;
let setInferenceConn: (v: any | null) => void = () => {};
let inferenceConnected: Accessor<boolean> = () => false;
let setInferenceConnected: (v: boolean) => void = () => {};

// fleet-state — retired (ADR-2604050900), compute_node now in hexflo-coordination
let fleetConnected: Accessor<boolean> = () => false;

// ---------------------------------------------------------------------------
// Table accessors — assigned inside createRoot by initConnectionStore
// ---------------------------------------------------------------------------

// hexflo-coordination tables
let swarms: Accessor<any[]> = () => [];
let swarmTasks: Accessor<any[]> = () => [];
let swarmAgents: Accessor<any[]> = () => [];
let hexfloMemory: Accessor<any[]> = () => [];
let registeredProjects: Accessor<any[]> = () => [];
let projectConfigs: Accessor<any[]> = () => [];
let skillRegistry: Accessor<any[]> = () => [];
let agentDefinitions: Accessor<any[]> = () => [];
let registryAgents: Accessor<any[]> = () => [];
let agentHeartbeats: Accessor<any[]> = () => [];
let agentInbox: Accessor<AgentInboxRow[]> = () => [];
let remoteAgents: Accessor<any[]> = () => [];

// inference-gateway tables
let inferenceProviders: Accessor<any[]> = () => [];
let inferenceRequests: Accessor<any[]> = () => [];

// fleet/compute_node — now served from hexflo-coordination (ADR-2604050900)
let fleetNodes: Accessor<any[]> = () => [];

// Aggregated connection status
let anyConnected: Accessor<boolean> = () => false;

// Export all accessors
export {
  hexfloConnected, agentRegistryConnected, inferenceConnected, fleetConnected,
  anyConnected,
  swarms, swarmTasks, swarmAgents, hexfloMemory,
  registeredProjects, projectConfigs, skillRegistry, agentDefinitions,
  registryAgents, agentHeartbeats, agentInbox, remoteAgents,
  inferenceProviders, inferenceRequests,
  fleetNodes,
};

// ---------------------------------------------------------------------------
// Generic connection helper
// ---------------------------------------------------------------------------

interface ConnectOpts {
  module: string;
  builder: { builder: () => any };
  setConn: (c: any) => void;
  setConnected: (v: boolean) => void;
  subscribeQueries: string[];
}

function connectModule(opts: ConnectOpts) {
  const tokenKey = TOKEN_KEY_PREFIX + opts.module;
  let savedToken: string | undefined = localStorage.getItem(tokenKey) ?? undefined;
  let retryCount = 0;

  function attempt() {
    console.log(`[stdb:${opts.module}] attempting connection to ${SPACETIMEDB_URI}/${opts.module}`);
    try {
      // SpacetimeDB SDK v2.0: DbConnection.builder() returns DbConnectionBuilder
      // Chain: .withUri() → .withDatabaseName() → .withToken() → .onConnect() → .build()
      const b = opts.builder.builder()
        .withUri(SPACETIMEDB_URI)
        .withDatabaseName(opts.module)
        .onConnect((ctx: any, _identity: any, token: string) => {
          console.log(`[stdb:${opts.module}] onConnect callback fired`);

          localStorage.setItem(tokenKey, token);
          retryCount = 0;

          // Subscribe first, then expose connection after subscription snapshot arrives.
          if (opts.subscribeQueries.length > 0) {
            ctx
              .subscriptionBuilder()
              .onApplied(() => {
                console.log(`[stdb:${opts.module}] subscription applied (${opts.subscribeQueries.length} queries)`);
                opts.setConn(ctx);
                opts.setConnected(true);
                console.log(`[stdb:${opts.module}] connection status set to true`);
              })
              .onError((_errCtx: any, err: Error) => {
                console.error(`[stdb:${opts.module}] subscription error:`, err);
                opts.setConn(ctx);
                opts.setConnected(true);
              })
              .subscribe(opts.subscribeQueries);
          } else {
            opts.setConn(ctx);
            opts.setConnected(true);
          }
        })
        .onDisconnect((_ctx: any, error?: Error) => {
          console.warn(`[stdb:${opts.module}] disconnected:`, error);
          // Set disconnected state immediately to prevent send() calls
          opts.setConnected(false);
          opts.setConn(null);
          // Use setTimeout to avoid issues during disconnect handler
          setTimeout(() => scheduleRetry(), 0);
        })
        .onConnectError((_ctx: any, err: Error) => {
          console.error(`[stdb:${opts.module}] connect error:`, err);
          console.error(`[stdb:${opts.module}] error details:`, err.message, err.stack);
          // Clear stale token on auth failure and retry without it
          if (savedToken && retryCount === 0) {
            console.warn(`[stdb:${opts.module}] clearing stale token and retrying...`);
            localStorage.removeItem(tokenKey);
            savedToken = undefined;
          }
          scheduleRetry();
        });

      if (savedToken) {
        b.withToken(savedToken);
      }

      b.build();
    } catch (err) {
      console.error(`[stdb:${opts.module}] build error:`, err);
      // Clear token on build error too (may be corrupted)
      if (savedToken) {
        localStorage.removeItem(tokenKey);
        savedToken = undefined;
      }
      scheduleRetry();
    }
  }

  function scheduleRetry() {
    const delay = Math.min(1000 * Math.pow(2, retryCount), 5_000); // max 5s backoff
    retryCount++;
    setTimeout(attempt, delay);
  }

  attempt();
}

// ---------------------------------------------------------------------------
// Initialization — call once from App.tsx composition root
// ---------------------------------------------------------------------------

let _storeInitialized = false;

/**
 * Initialize all connection signals and table accessors inside a createRoot.
 * Must be called before any other store init or initConnections().
 */
export function initConnectionStore() {
  if (_storeInitialized) return;
  _storeInitialized = true;

  createRoot(() => {
    // Connection state signals
    const [_hexfloConn, _setHexfloConn] = createSignal<any | null>(null);
    const [_hexfloConnected, _setHexfloConnected] = createSignal(false);
    const [_inferenceConn, _setInferenceConn] = createSignal<any | null>(null);
    const [_inferenceConnected, _setInferenceConnected] = createSignal(false);
    // ADR-2604050900: fleet-state retired; fleetConnected mirrors hexfloConnected

    // Assign to module-level variables
    hexfloConn = _hexfloConn;
    setHexfloConn = _setHexfloConn;
    hexfloConnected = _hexfloConnected;
    setHexfloConnected = _setHexfloConnected;
    agentRegistryConnected = _hexfloConnected; // delegate to hexflo (ADR-058)
    inferenceConn = _inferenceConn;
    setInferenceConn = _setInferenceConn;
    inferenceConnected = _inferenceConnected;
    setInferenceConnected = _setInferenceConnected;
    fleetConnected = _hexfloConnected; // fleet data now from hexflo-coordination

    // Table accessors (useTable creates createEffect inside — needs reactive owner)
    swarms = useTable(() => _hexfloConn()?.db.swarm as SpacetimeDBTableHandle<any> | undefined);
    swarmTasks = useTable(() => _hexfloConn()?.db.swarm_task as SpacetimeDBTableHandle<any> | undefined);
    swarmAgents = useTable(() => _hexfloConn()?.db.swarm_agent as SpacetimeDBTableHandle<any> | undefined);
    hexfloMemory = useTable(() => _hexfloConn()?.db.hexflo_memory as SpacetimeDBTableHandle<any> | undefined);
    registeredProjects = useTable(() => _hexfloConn()?.db.project as SpacetimeDBTableHandle<any> | undefined);
    projectConfigs = useTable(() => _hexfloConn()?.db.project_config as SpacetimeDBTableHandle<any> | undefined);
    skillRegistry = useTable(() => _hexfloConn()?.db.skill_registry as SpacetimeDBTableHandle<any> | undefined);
    agentDefinitions = useTable(() => _hexfloConn()?.db.agent_definition as SpacetimeDBTableHandle<any> | undefined);
    registryAgents = useTable(() => _hexfloConn()?.db.hex_agent as SpacetimeDBTableHandle<any> | undefined);
    agentHeartbeats = () => []; // Heartbeat data inline on hex_agent.lastHeartbeat (ADR-058)
    agentInbox = useTable(() => _hexfloConn()?.db.agent_inbox as SpacetimeDBTableHandle<AgentInboxRow> | undefined);
    remoteAgents = useTable(() => _hexfloConn()?.db.remote_agent as SpacetimeDBTableHandle<any> | undefined);

    // inference-gateway tables
    inferenceProviders = useTable(() => _inferenceConn()?.db.inference_provider as SpacetimeDBTableHandle<any> | undefined);
    inferenceRequests = useTable(() => _inferenceConn()?.db.inference_request as SpacetimeDBTableHandle<any> | undefined);

    // fleet/compute_node — now served from hexflo-coordination (ADR-2604050900)
    fleetNodes = useTable(() => _hexfloConn()?.db.compute_node as SpacetimeDBTableHandle<any> | undefined);

    // Aggregated connection status
    anyConnected = () => _hexfloConnected() || _inferenceConnected();
  });
}

let _connectionsInitialized = false;

/**
 * Initialize all SpacetimeDB module connections.
 * Safe to call multiple times (idempotent).
 * Must be called after initConnectionStore().
 */
export function initConnections() {
  if (_connectionsInitialized) return;
  _connectionsInitialized = true;

  if (!SPACETIMEDB_URI) {
    console.warn("[stdb] No SpacetimeDB URI - connections disabled");
    return;
  }

  // hexflo-coordination (main coordination database)
  connectModule({
    module: "hex",
    builder: HexfloDbConnection,
    setConn: setHexfloConn,
    setConnected: setHexfloConnected,
    subscribeQueries: [
      "SELECT * FROM swarm",
      "SELECT * FROM swarm_task",
      "SELECT * FROM swarm_agent",
      "SELECT * FROM hexflo_memory",
      "SELECT * FROM project",
      "SELECT * FROM project_config",
      "SELECT * FROM skill_registry",
      "SELECT * FROM agent_definition",
      "SELECT * FROM hex_agent",
      "SELECT * FROM agent_inbox",
      "SELECT * FROM remote_agent",
      "SELECT * FROM compute_node",
    ],
  });

  // inference-gateway
  connectModule({
    module: "inference-gateway",
    builder: InferenceGatewayDbConnection,
    setConn: setInferenceConn,
    setConnected: setInferenceConnected,
    subscribeQueries: [
      "SELECT * FROM inference_provider",
      "SELECT * FROM inference_request",
    ],
  });
}

// ---------------------------------------------------------------------------
// Reducer access (for mutation calls)
// ---------------------------------------------------------------------------

/** Get the hexflo-coordination connection for calling reducers. */
export function getHexfloConn() { return hexfloConn(); }
/** Get the agent-registry connection for calling reducers (delegates to hexflo — ADR-058). */
export function getAgentRegistryConn() { return hexfloConn(); }
/** Get the inference-gateway connection for calling reducers. */
export function getInferenceConn() { return inferenceConn(); }
/** Get the fleet/compute_node connection (now served from hexflo-coordination — ADR-2604050900). */
export function getFleetConn() { return hexfloConn(); }
