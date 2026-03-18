/**
 * Checkpoint Port
 *
 * Contract for persisting and recovering swarm state snapshots.
 * Adapters may back this with file-based JSON (MVP), AgentDB
 * (production), or in-memory storage (tests).
 *
 * Dependency: domain/checkpoint-types.ts only.
 */

import type { CheckpointEntry } from '../domain/checkpoint-types.js';

export type { CheckpointEntry, TaskSnapshot, FeatureProgress, FeaturePhase } from '../domain/checkpoint-types.js';

export interface ICheckpointPort {
  /** Persist a checkpoint snapshot. Must not throw on write failure. */
  checkpoint(entry: CheckpointEntry): Promise<void>;

  /** Load the most recent checkpoint for a project. Returns null if none exists. */
  recover(projectId: string): Promise<CheckpointEntry | null>;

  /** List all checkpoints for a project, ordered newest-first. */
  list(projectId: string): Promise<CheckpointEntry[]>;

  /** Remove old checkpoints, keeping only the N most recent. Returns count deleted. */
  prune(projectId: string, keepCount: number): Promise<number>;
}

/** Port interface for the checkpoint orchestrator (use case layer implements this). */
export interface ICheckpointOrchestrator {
  /** Capture a manual checkpoint of current swarm state. */
  manualCheckpoint(): Promise<CheckpointEntry>;
  /** Recover the most recent checkpoint. READ-ONLY — does not modify swarm state. */
  recover(): Promise<CheckpointEntry | null>;
  /** Auto-checkpoint on task status transitions. */
  onTaskTransition(taskId: string, newStatus: string): Promise<void>;
  /** Remove old checkpoints, keeping only the most recent N. */
  pruneOld(keepCount?: number): Promise<number>;
}
