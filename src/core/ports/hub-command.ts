/**
 * Hub Command Port
 *
 * Defines the contract for bidirectional communication between the
 * hex-hub and connected projects. The hub can issue commands to
 * projects (spawn agents, run analysis, etc.) and projects respond
 * with results.
 *
 * This is a PRIMARY (driving) port — the hub drives project behavior
 * through commands. The DashboardAdapter implements the listener side;
 * the MCP adapter and browser UI implement the sender side.
 */

import type { AgentRole } from './swarm.js';

// ─── Command Types ──────────────────────────────────────

export type HubCommandType =
  | 'spawn-agent'
  | 'terminate-agent'
  | 'create-task'
  | 'cancel-task'
  | 'run-analyze'
  | 'run-build'
  | 'run-validate'
  | 'run-generate'
  | 'run-summarize'
  | 'run-claude'
  | 'ping';

// ─── Command Payloads ───────────────────────────────────

export interface SpawnAgentPayload {
  name: string;
  role: AgentRole;
  taskId?: string;
  prompt?: string;
}

export interface TerminateAgentPayload {
  agentId: string;
}

export interface CreateTaskPayload {
  title: string;
  agentRole: AgentRole;
  adapter?: string;
  language?: string;
}

export interface CancelTaskPayload {
  taskId: string;
}

export interface RunAnalyzePayload {
  rootPath?: string;
  checks?: ('dead-exports' | 'hex-boundaries' | 'circular-deps' | 'full')[];
}

export interface RunBuildPayload {
  rootPath?: string;
}

export interface RunValidatePayload {
  rootPath?: string;
}

export interface RunGeneratePayload {
  adapter: string;
  portInterface: string;
  language?: string;
}

export interface RunSummarizePayload {
  filePath?: string;
  level?: 'L0' | 'L1' | 'L2' | 'L3';
}

export type HubCommandPayload =
  | SpawnAgentPayload
  | TerminateAgentPayload
  | CreateTaskPayload
  | CancelTaskPayload
  | RunAnalyzePayload
  | RunBuildPayload
  | RunValidatePayload
  | RunGeneratePayload
  | RunSummarizePayload
  | Record<string, never>;  // ping has no payload

// ─── Command Envelope ───────────────────────────────────

export interface HubCommand {
  /** Unique command ID for correlation (UUID) */
  commandId: string;
  /** Target project ID */
  projectId: string;
  /** Command type */
  type: HubCommandType;
  /** Type-specific payload */
  payload: HubCommandPayload;
  /** ISO timestamp when command was issued */
  issuedAt: string;
  /** Who issued the command: 'browser', 'mcp', 'cli' */
  source: 'browser' | 'mcp' | 'cli';
}

// ─── Command Result ─────────────────────────────────────

export type HubCommandStatus = 'pending' | 'dispatched' | 'running' | 'completed' | 'failed';

export interface HubCommandResult {
  /** Correlates to HubCommand.commandId */
  commandId: string;
  /** Final status */
  status: HubCommandStatus;
  /** Result data (command-type specific) */
  data?: unknown;
  /** Error message if failed */
  error?: string;
  /** ISO timestamp when result was produced */
  completedAt: string;
}

// ─── Driving Port (Hub → Project) ───────────────────────

/**
 * Implemented by the DashboardAdapter on the project side.
 * Receives commands from the hub and executes them locally.
 */
export interface IHubCommandReceiverPort {
  /** Start listening for commands from the hub */
  startListening(): Promise<void>;

  /** Stop listening and disconnect */
  stopListening(): Promise<void>;

  /** Whether the receiver is currently connected and listening */
  isListening(): boolean;

  /** Register a handler for a specific command type */
  onCommand(type: HubCommandType, handler: HubCommandHandler): void;

  /** Remove a handler */
  offCommand(type: HubCommandType): void;
}

export type HubCommandHandler = (command: HubCommand) => Promise<HubCommandResult>;

// ─── Sending Port (CLI/MCP/Browser → Hub) ───────────────

/**
 * Implemented by the MCP adapter and browser UI.
 * Sends commands to a specific project via the hub.
 */
export interface IHubCommandSenderPort {
  /** Send a command to a project and wait for the result */
  sendCommand(command: Omit<HubCommand, 'commandId' | 'issuedAt'>): Promise<HubCommandResult>;

  /** Send a command without waiting (fire-and-forget) */
  dispatchCommand(command: Omit<HubCommand, 'commandId' | 'issuedAt'>): Promise<string>;

  /** Check the status/result of a previously dispatched command */
  getCommandStatus(commandId: string): Promise<HubCommandResult | null>;

  /** List recent commands for a project */
  listCommands(projectId: string, limit?: number): Promise<HubCommand[]>;
}
