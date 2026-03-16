/**
 * Notification & Developer Awareness Ports
 *
 * Keeps the developer informed of agent activity, quality gate results,
 * and decision points requiring human input. Inspired by ruflo's status
 * system but designed for hex-intf's feedback loop workflow.
 */

// ─── Notification Levels ─────────────────────────────────

export type NotificationLevel =
  | 'trace'      // Agent internal steps (only in verbose mode)
  | 'info'       // Normal progress: "Compiling adapter..."
  | 'success'    // Quality gate passed: "All 12 tests passing"
  | 'warning'    // Non-blocking issue: "3 lint warnings remaining"
  | 'error'      // Blocking failure: "Build failed: missing type"
  | 'decision'   // Needs human input: "2 approaches possible, which?"
  | 'milestone';  // Major checkpoint: "Phase 2 complete, 4/6 adapters done"

export type NotificationChannel =
  | 'terminal'    // Inline CLI output (always on)
  | 'status_bar'  // Persistent status line (like ruflo)
  | 'toast'       // OS-level notification (macOS/Linux)
  | 'webhook'     // HTTP POST to external service
  | 'file_log'    // Append to .hex-intf/activity.log
  | 'event_bus';  // Internal pub/sub for other agents

// ─── Core Types ──────────────────────────────────────────

export interface Notification {
  id: string;
  timestamp: number;
  level: NotificationLevel;
  source: NotificationSource;
  title: string;
  detail?: string;
  context?: NotificationContext;
  actions?: NotificationAction[];
  ttl?: number; // Auto-dismiss after ms (0 = persistent)
}

export interface NotificationSource {
  agentName: string;
  agentType: string;
  phase: 'plan' | 'execute' | 'integrate' | 'package';
  adapter?: string;    // Which adapter boundary
  worktree?: string;   // Which git worktree
}

export interface NotificationContext {
  // Quality gate context
  qualityScore?: number;
  iterationCount?: number;
  maxIterations?: number;

  // Progress context
  stepsCompleted?: number;
  stepsTotal?: number;
  percentComplete?: number;

  // Token context
  tokensUsed?: number;
  tokenBudget?: number;

  // File context
  filesChanged?: string[];
  linesAdded?: number;
  linesRemoved?: number;
}

export interface NotificationAction {
  label: string;
  type: 'approve' | 'reject' | 'choose' | 'snooze' | 'escalate';
  payload?: Record<string, unknown>;
}

// ─── Progress Tracking ───────────────────────────────────

export interface ProgressReport {
  swarmId: string;
  phase: string;
  agents: AgentProgress[];
  overallPercent: number;
  estimatedRemaining?: number; // ms
  blockers: Blocker[];
}

export interface AgentProgress {
  agentName: string;
  adapter: string;
  status: 'queued' | 'running' | 'blocked' | 'done' | 'failed';
  currentStep: string;
  qualityScore?: number;
  iteration: number;
  maxIterations: number;
  lastActivity: number; // timestamp
}

export interface Blocker {
  agentName: string;
  type: 'compile_error' | 'test_failure' | 'merge_conflict' | 'decision_needed' | 'timeout';
  description: string;
  since: number; // timestamp
  suggestedAction?: string;
}

// ─── Status Line Format ──────────────────────────────────

export interface StatusLine {
  /** Compact one-line summary for persistent display */
  compact: string;
  /** Expanded multi-line for verbose mode */
  expanded: string[];
  /** ANSI color codes for terminal rendering */
  ansiCompact: string;
}

// ─── Decision Request ────────────────────────────────────

export interface DecisionRequest {
  id: string;
  agentName: string;
  question: string;
  options: DecisionOption[];
  deadline?: number;        // Auto-select default after ms
  defaultOption?: string;   // Option ID to auto-select
  context: string;          // What the agent was doing when it hit this
}

export interface DecisionOption {
  id: string;
  label: string;
  description: string;
  risk: 'low' | 'medium' | 'high';
  estimatedImpact: string;
}

export interface DecisionResponse {
  requestId: string;
  selectedOption: string;
  respondedBy: 'human' | 'auto_timeout' | 'escalation_agent';
  timestamp: number;
}

// ─── Notification Preferences ────────────────────────────

export interface NotificationPreferences {
  channels: NotificationChannel[];
  minLevel: NotificationLevel;
  quietMode: boolean;          // Only errors and decisions
  progressInterval: number;    // ms between progress updates (default: 5000)
  decisionTimeout: number;     // ms before auto-selecting default (default: 30000)
  groupByAdapter: boolean;     // Group notifications by adapter boundary
  showTokenUsage: boolean;     // Include token counts in progress
}

// ─── Input Port (Primary / Driving) ──────────────────────

export type NotificationListener = (notification: Notification) => void;

export interface INotificationQueryPort {
  /** Get current progress for all agents */
  getProgress(): Promise<ProgressReport>;

  /** Get pending decisions awaiting human input */
  getPendingDecisions(): Promise<DecisionRequest[]>;

  /** Respond to a decision request */
  respondToDecision(response: DecisionResponse): Promise<void>;

  /** Get recent notifications */
  getRecent(limit: number, minLevel?: NotificationLevel): Promise<Notification[]>;

  /** Update notification preferences */
  setPreferences(prefs: Partial<NotificationPreferences>): Promise<void>;

  /** Register a callback for every emitted notification (used by dashboard SSE) */
  addListener(fn: NotificationListener): void;
}

// ─── Output Port (Secondary / Driven) ────────────────────

export interface INotificationEmitPort {
  /** Emit a notification to configured channels */
  notify(notification: Omit<Notification, 'id' | 'timestamp'>): Promise<void>;

  /** Update the persistent status line */
  updateStatusLine(status: StatusLine): Promise<void>;

  /** Request a decision from the developer */
  requestDecision(request: Omit<DecisionRequest, 'id'>): Promise<DecisionResponse>;

  /** Emit a progress report */
  reportProgress(report: ProgressReport): Promise<void>;

  /** Register a channel for notifications */
  registerChannel(channel: NotificationChannel, config?: Record<string, unknown>): Promise<void>;
}
