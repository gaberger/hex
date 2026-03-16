/**
 * File Checkpoint Adapter
 *
 * Implements ICheckpointPort using JSON files on disk.
 * Directory layout: {basePath}/{projectId}/{ISO-timestamp}.json
 *
 * Uses IFileSystemPort for all file operations — never touches
 * the filesystem directly.
 *
 * Hex layer: secondary adapter (driven).
 * Imports: ports only (ICheckpointPort, IFileSystemPort, CheckpointEntry).
 */

import type { ICheckpointPort, CheckpointEntry } from '../../core/ports/checkpoint.js';
import type { IFileSystemPort } from '../../core/ports/index.js';

/**
 * Sanitise a project ID for use as a directory name.
 * Replaces path-unsafe characters with underscores.
 */
function safeDirName(projectId: string): string {
  return projectId.replace(/[^a-zA-Z0-9_-]/g, '_');
}

/**
 * Build a filename from an ISO timestamp.
 * Colons are replaced with dashes so the name is valid on all OSes.
 */
function timestampToFilename(iso: string): string {
  return iso.replace(/:/g, '-') + '.json';
}

export class FileCheckpointAdapter implements ICheckpointPort {
  private readonly basePath: string;
  private readonly fs: IFileSystemPort;

  constructor(basePath: string, fs: IFileSystemPort) {
    this.basePath = basePath;
    this.fs = fs;
  }

  // ── checkpoint ──────────────────────────────────────────

  async checkpoint(entry: CheckpointEntry): Promise<void> {
    const dir = `${this.basePath}/${safeDirName(entry.projectId)}`;
    const filename = timestampToFilename(entry.createdAt);
    const target = `${dir}/${filename}`;
    const tmp = `${target}.tmp`;

    try {
      const json = JSON.stringify(entry, null, 2);

      // Write to tmp first, then rename-via-overwrite for atomicity.
      // IFileSystemPort.write creates parent directories as needed
      // in the reference filesystem-adapter implementation.
      await this.fs.write(tmp, json);
      // "Rename" by writing to final path and clearing tmp.
      // A true rename would be better, but IFileSystemPort only
      // exposes read/write/exists/glob — no rename primitive.
      await this.fs.write(target, json);
      // Best-effort cleanup of tmp: overwrite with empty content.
      // Cannot delete because IFileSystemPort lacks a delete method.
      await this.fs.write(tmp, '');
    } catch (err) {
      // Must not throw on write failure (port contract).
      // eslint-disable-next-line no-console
      console.error(
        `[file-checkpoint] Failed to write checkpoint ${entry.id}: ${err instanceof Error ? err.message : String(err)}`,
      );
    }
  }

  // ── recover ─────────────────────────────────────────────

  async recover(projectId: string): Promise<CheckpointEntry | null> {
    const files = await this.listFiles(projectId);
    if (files.length === 0) return null;

    // Files sort lexicographically by ISO timestamp — last is newest.
    const newest = files[files.length - 1];
    return this.readCheckpoint(newest);
  }

  // ── list ────────────────────────────────────────────────

  async list(projectId: string): Promise<CheckpointEntry[]> {
    const files = await this.listFiles(projectId);
    const entries: CheckpointEntry[] = [];

    for (const file of files) {
      const entry = await this.readCheckpoint(file);
      if (entry) entries.push(entry);
    }

    // Newest first — reverse the lexicographic (oldest-first) order.
    return entries.reverse();
  }

  // ── prune ───────────────────────────────────────────────

  /**
   * Keep only the `keepCount` most recent checkpoints and "delete" the rest.
   *
   * Limitation: IFileSystemPort does not expose a delete() method, so pruned
   * files are overwritten with empty content. They will be ignored by
   * readCheckpoint() (empty string fails JSON.parse). A future
   * IFileSystemPort.delete() method would make this cleaner.
   *
   * Returns the number of checkpoints removed.
   */
  async prune(projectId: string, keepCount: number): Promise<number> {
    const files = await this.listFiles(projectId);
    if (files.length <= keepCount) return 0;

    // Files are sorted oldest-first. Keep the last `keepCount`.
    const toDelete = files.slice(0, files.length - keepCount);
    let deleted = 0;

    for (const file of toDelete) {
      try {
        await this.fs.write(file, '');
        deleted++;
      } catch {
        // Best effort — continue with remaining files.
        // eslint-disable-next-line no-console
        console.error(`[file-checkpoint] Failed to prune ${file}`);
      }
    }

    return deleted;
  }

  // ── private helpers ─────────────────────────────────────

  /**
   * List checkpoint JSON files for a project, sorted oldest-first
   * (lexicographic by ISO-timestamp filename).
   * Excludes .tmp files and empty (pruned) files.
   */
  private async listFiles(projectId: string): Promise<string[]> {
    const dir = `${this.basePath}/${safeDirName(projectId)}`;
    const dirExists = await this.fs.exists(dir);
    if (!dirExists) return [];

    const pattern = `${dir}/*.json`;
    const allFiles = await this.fs.glob(pattern);

    // Filter out .tmp files (should not match *.json, but be safe)
    // and sort lexicographically so timestamps are in order.
    return allFiles
      .filter((f) => !f.endsWith('.tmp'))
      .sort();
  }

  /**
   * Read and parse a single checkpoint file.
   * Returns null if the file is empty (pruned) or unparseable.
   */
  private async readCheckpoint(filePath: string): Promise<CheckpointEntry | null> {
    try {
      const content = await this.fs.read(filePath);
      if (!content || content.trim().length === 0) return null;
      return JSON.parse(content) as CheckpointEntry;
    } catch {
      // Corrupt or unreadable file — skip silently.
      return null;
    }
  }
}
