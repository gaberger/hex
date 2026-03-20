/**
 * connection.ts — Singleton SpacetimeDB connection store for hex-nexus dashboard.
 *
 * Creates a DbConnection to the hexflo-coordination module, subscribes to
 * all core tables, and exports typed SolidJS signal accessors for each table.
 *
 * Usage:
 *   import { swarms, tasks, agents, connected } from "../stores/connection";
 *   // In a component:  <For each={swarms()}>{(s) => ...}</For>
 */
import {
  createSignal,
  createEffect,
  type Accessor,
} from "solid-js";
import { useTable, type SpacetimeDBTableHandle } from "../hooks/useTable";
import {
  DbConnection as HexfloDbConnection,
} from "../spacetimedb/hexflo-coordination/index";
import {
  DbConnection as AgentRegistryDbConnection,
} from "../spacetimedb/agent-registry/index";
import {
  DbConnection as InferenceGatewayDbConnection,
} from "../spacetimedb/inference-gateway/index";
import {
  DbConnection as FleetStateDbConnection,
} from "../spacetimedb/fleet-state/index";

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

const SPACETIMEDB_URI = "ws://localhost:3000";
const TOKEN_KEY_PREFIX = "stdb_token_";

// ---------------------------------------------------------------------------
// Connection state signals
// ---------------------------------------------------------------------------

// hexflo-coordination
const [hexfloConn, setHexfloConn] = createSignal<any | null>(null);
const [hexfloConnected, setHexfloConnected] = createSignal(false);

// agent-registry
const [agentRegistryConn, setAgentRegistryConn] = createSignal<any | null>(null);
const [agentRegistryConnected, setAgentRegistryConnected] = createSignal(false);

// inference-gateway
const [inferenceConn, setInferenceConn] = createSignal<any | null>(null);
const [inferenceConnected, setInferenceConnected] = createSignal(false);

// fleet-state
const [fleetConn, setFleetConn] = createSignal<any | null>(null);
const [fleetConnected, setFleetConnected] = createSignal(false);

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
  const savedToken = localStorage.getItem(tokenKey) ?? undefined;
  let retryCount = 0;

  function attempt() {
    try {
      // SpacetimeDB SDK v2.0: DbConnection.builder() returns DbConnectionBuilder
      // Chain: .withUri() → .withDatabaseName() → .withToken() → .onConnect() → .build()
      const b = opts.builder.builder()
        .withUri(SPACETIMEDB_URI)
        .withDatabaseName(opts.module)
        .onConnect((ctx: any, _identity: any, token: string) => {
          localStorage.setItem(tokenKey, token);
          opts.setConn(ctx);
          opts.setConnected(true);
          retryCount = 0;

          // Subscribe to all tables for this module
          if (opts.subscribeQueries.length > 0) {
            ctx
              .subscriptionBuilder()
              .onApplied(() => {
                console.log(`[stdb:${opts.module}] subscription applied`);
              })
              .onError((_errCtx: any, err: Error) => {
                console.error(`[stdb:${opts.module}] subscription error:`, err);
              })
              .subscribe(opts.subscribeQueries);
          }
        })
        .onDisconnect((_ctx: any, _error?: Error) => {
          opts.setConnected(false);
          opts.setConn(null);
          scheduleRetry();
        })
        .onConnectError((_ctx: any, err: Error) => {
          console.error(`[stdb:${opts.module}] connect error:`, err);
          scheduleRetry();
        });

      if (savedToken) {
        b.withToken(savedToken);
      }

      b.build();
    } catch (err) {
      console.error(`[stdb:${opts.module}] build error:`, err);
      scheduleRetry();
    }
  }

  function scheduleRetry() {
    const delay = Math.min(1000 * Math.pow(2, retryCount), 30_000);
    retryCount++;
    setTimeout(attempt, delay);
  }

  attempt();
}

// ---------------------------------------------------------------------------
// Table accessors (reactive signals)
// ---------------------------------------------------------------------------

// hexflo-coordination tables
export const swarms: Accessor<any[]> = useTable(
  () => hexfloConn()?.db.swarm as SpacetimeDBTableHandle<any> | undefined,
);
export const swarmTasks: Accessor<any[]> = useTable(
  () => hexfloConn()?.db.swarm_task as SpacetimeDBTableHandle<any> | undefined,
);
export const swarmAgents: Accessor<any[]> = useTable(
  () => hexfloConn()?.db.swarm_agent as SpacetimeDBTableHandle<any> | undefined,
);
export const hexfloMemory: Accessor<any[]> = useTable(
  () => hexfloConn()?.db.hexflo_memory as SpacetimeDBTableHandle<any> | undefined,
);

// agent-registry tables
export const registryAgents: Accessor<any[]> = useTable(
  () => agentRegistryConn()?.db.agent as SpacetimeDBTableHandle<any> | undefined,
);
export const agentHeartbeats: Accessor<any[]> = useTable(
  () => agentRegistryConn()?.db.agent_heartbeat as SpacetimeDBTableHandle<any> | undefined,
);

// inference-gateway tables
export const inferenceProviders: Accessor<any[]> = useTable(
  () => inferenceConn()?.db.inference_provider as SpacetimeDBTableHandle<any> | undefined,
);
export const inferenceRequests: Accessor<any[]> = useTable(
  () => inferenceConn()?.db.inference_request as SpacetimeDBTableHandle<any> | undefined,
);

// fleet-state tables
export const fleetNodes: Accessor<any[]> = useTable(
  () => fleetConn()?.db.compute_node as SpacetimeDBTableHandle<any> | undefined,
);

// Aggregated connection status
export { hexfloConnected, agentRegistryConnected, inferenceConnected, fleetConnected };

export const allConnected: Accessor<boolean> = () =>
  hexfloConnected() && agentRegistryConnected() && inferenceConnected() && fleetConnected();

export const anyConnected: Accessor<boolean> = () =>
  hexfloConnected() || agentRegistryConnected() || inferenceConnected() || fleetConnected();

// ---------------------------------------------------------------------------
// Initialization — call once at app startup
// ---------------------------------------------------------------------------

let initialized = false;

/**
 * Initialize all SpacetimeDB module connections.
 * Safe to call multiple times (idempotent).
 */
export function initConnections() {
  if (initialized) return;
  initialized = true;

  // hexflo-coordination: swarms, tasks, agents, memory
  connectModule({
    module: "hexflo-coordination",
    builder: HexfloDbConnection,
    setConn: setHexfloConn,
    setConnected: setHexfloConnected,
    subscribeQueries: [
      "SELECT * FROM swarm",
      "SELECT * FROM swarm_task",
      "SELECT * FROM swarm_agent",
      "SELECT * FROM hexflo_memory",
    ],
  });

  // agent-registry: agents, heartbeats
  connectModule({
    module: "agent-registry",
    builder: AgentRegistryDbConnection,
    setConn: setAgentRegistryConn,
    setConnected: setAgentRegistryConnected,
    subscribeQueries: [
      "SELECT * FROM agent",
      "SELECT * FROM agent_heartbeat",
    ],
  });

  // inference-gateway: providers, requests
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

  // fleet-state: compute nodes
  connectModule({
    module: "fleet-state",
    builder: FleetStateDbConnection,
    setConn: setFleetConn,
    setConnected: setFleetConnected,
    subscribeQueries: [
      "SELECT * FROM compute_node",
    ],
  });
}

// ---------------------------------------------------------------------------
// Reducer access (for mutation calls)
// ---------------------------------------------------------------------------

/** Get the hexflo-coordination connection for calling reducers. */
export function getHexfloConn() { return hexfloConn(); }
/** Get the agent-registry connection for calling reducers. */
export function getAgentRegistryConn() { return agentRegistryConn(); }
/** Get the inference-gateway connection for calling reducers. */
export function getInferenceConn() { return inferenceConn(); }
/** Get the fleet-state connection for calling reducers. */
export function getFleetConn() { return fleetConn(); }
