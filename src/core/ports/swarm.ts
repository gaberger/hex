/**
 * Swarm Coordination Port
 *
 * Abstracts ruflo's swarm orchestration behind a hex port interface.
 * ruflo is a REQUIRED dependency of hex — this port is always
 * backed by the ruflo adapter in production. The port exists so that
 * tests can mock swarm behavior without starting a real daemon.
 */

import type { Language, WorkplanStep } from './index.js';

// ─── Swarm Types ─────────────────────────────────────────

export type SwarmTopology = 'hierarchical' | 'mesh' | 'hierarchical-mesh';
export type SwarmStrategy = 'specialized' | 'generalist' | 'adaptive';
export type AgentRole = 'planner' | 'coder' | 'tester' | 'reviewer' | 'integrator' | 'monitor';

export interface SwarmConfig {
  topology: SwarmTopology;
  maxAgents: number;
  strategy: SwarmStrategy;
  consensus: 'raft' | 'pbft';
  memoryNamespace: string;
}

export interface SwarmStatus {
  id: string;
  topology: SwarmTopology;
  agentCount: number;
  activeTaskCount: number;
  completedTaskCount: number;
  status: 'initializing' | 'running' | 'idle' | 'completed' | 'failed';
}

export interface SwarmTask {
  id: string;
  title: string;
  assignee?: string;
  agentRole: AgentRole;
  adapter?: string;        // Which adapter boundary this task targets
  worktreeBranch?: string;  // Git worktree branch for isolation
  language?: Language;
  status: 'pending' | 'assigned' | 'running' | 'completed' | 'failed';
  result?: string;
  commitHash?: string;
}

export interface SwarmAgent {
  id: string;
  name: string;
  role: AgentRole;
  status: 'spawning' | 'active' | 'idle' | 'terminated';
  currentTask?: string;
  worktree?: string;
}

export interface SwarmMemoryEntry {
  key: string;
  value: string;
  namespace: string;
  tags?: string[];
  ttl?: number;
}

// ─── AgentDB Types ───────────────────────────────────────

export interface AgentDBPattern {
  id: string;
  name: string;
  category: string;
  content: string;
  confidence: number;
  accessCount: number;
  createdAt: string;
  updatedAt: string;
  tags?: string[];
}

export interface AgentDBFeedback {
  patternId: string;
  outcome: 'success' | 'failure' | 'partial';
  score: number;
  context?: string;
  details?: string;
}

export interface AgentDBSession {
  sessionId: string;
  agentName: string;
  startedAt: string;
  status: 'active' | 'ended';
  metadata?: Record<string, unknown>;
}

export interface AgentDBProgressReport {
  swarmId: string;
  tasks: SwarmTask[];
  agents: SwarmAgent[];
  patterns: { total: number; recentlyUsed: number };
  sessions: AgentDBSession[];
  overallPercent: number;
  phase: string;
}

// ─── Output Port (Secondary / Driven) ────────────────────

export interface ISwarmPort {
  /** Check if the swarm daemon is reachable (result cached for 60s) */
  healthCheck(): Promise<boolean>;

  /** Initialize a new swarm with the given configuration */
  init(config: SwarmConfig): Promise<SwarmStatus>;

  /** Get current swarm status */
  status(): Promise<SwarmStatus>;

  /** Shutdown the swarm */
  shutdown(): Promise<void>;

  /** Create a task in the swarm registry */
  createTask(task: Omit<SwarmTask, 'id' | 'status'>): Promise<SwarmTask>;

  /** Mark a task as complete with optional commit hash */
  completeTask(taskId: string, result: string, commitHash?: string): Promise<void>;

  /** List all tasks with optional status filter */
  listTasks(statusFilter?: SwarmTask['status']): Promise<SwarmTask[]>;

  /** Spawn an agent with a specific role */
  spawnAgent(name: string, role: AgentRole, taskId?: string): Promise<SwarmAgent>;

  /** Terminate an agent */
  terminateAgent(agentId: string): Promise<void>;

  /** List active agents */
  listAgents(): Promise<SwarmAgent[]>;

  /** Store a value in swarm memory (persists across sessions) */
  memoryStore(entry: SwarmMemoryEntry): Promise<void>;

  /** Retrieve a value from swarm memory */
  memoryRetrieve(key: string, namespace: string): Promise<string | null>;

  /** Search swarm memory */
  memorySearch(query: string, namespace?: string): Promise<SwarmMemoryEntry[]>;

  // ─── AgentDB: Pattern Learning ─────────────────────────

  /** Store a learned pattern (successful approach, code template, decision) */
  patternStore(pattern: Omit<AgentDBPattern, 'id' | 'accessCount' | 'createdAt' | 'updatedAt'>): Promise<AgentDBPattern>;

  /** Search for relevant patterns by query and optional category */
  patternSearch(query: string, category?: string, limit?: number): Promise<AgentDBPattern[]>;

  /** Record outcome feedback for a pattern to refine confidence scores */
  patternFeedback(feedback: AgentDBFeedback): Promise<void>;

  // ─── AgentDB: Session Tracking ─────────────────────────

  /** Start a tracked session for an agent (enables progress visibility) */
  sessionStart(agentName: string, metadata?: Record<string, unknown>): Promise<AgentDBSession>;

  /** End a tracked session */
  sessionEnd(sessionId: string): Promise<void>;

  // ─── AgentDB: Hierarchical Memory ─────────────────────

  /** Store a memory entry with hierarchical context (layer > namespace > key) */
  hierarchicalStore(layer: string, namespace: string, key: string, value: string, tags?: string[]): Promise<void>;

  /** Recall memories from a hierarchical path with optional depth traversal */
  hierarchicalRecall(layer: string, namespace?: string, key?: string): Promise<SwarmMemoryEntry[]>;

  // ─── AgentDB: Intelligence ─────────────────────────────

  /** Consolidate and deduplicate stored patterns/memories */
  consolidate(): Promise<{ merged: number; removed: number }>;

  /** Synthesize context from multiple memory sources for a given query */
  contextSynthesize(query: string, sources?: string[]): Promise<string>;

  // ─── AgentDB: Aggregate Progress ──────────────────────

  /** Get a full progress report with tasks, agents, patterns, and sessions */
  getProgressReport(): Promise<AgentDBProgressReport>;
}

// ─── Input Port (Primary / Driving) ──────────────────────

export interface ISwarmOrchestrationPort {
  /** Plan and execute a workplan using swarm agents in parallel worktrees */
  orchestrate(steps: WorkplanStep[], config?: Partial<SwarmConfig>): Promise<SwarmStatus>;

  /** Get a formatted progress report of the current swarm */
  getProgress(): Promise<AgentDBProgressReport>;
}
