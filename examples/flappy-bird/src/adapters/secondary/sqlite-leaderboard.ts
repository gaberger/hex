/**
 * SQLite Leaderboard — Secondary Adapter
 * Implements ILeaderboardPort using bun:sqlite for persistent score storage.
 */

import { Database } from 'bun:sqlite';
import type { ILeaderboardPort, LeaderboardEntry } from '../../core/ports/index.js';

export class SqliteLeaderboard implements ILeaderboardPort {
  private readonly db: Database;

  constructor(dbPath: string = 'leaderboard.db') {
    this.db = new Database(dbPath);
    this.db.run(`
      CREATE TABLE IF NOT EXISTS scores (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        name TEXT NOT NULL,
        score INTEGER NOT NULL,
        timestamp TEXT NOT NULL DEFAULT (datetime('now'))
      )
    `);
  }

  async saveScore(name: string, score: number): Promise<void> {
    this.db.run('INSERT INTO scores (name, score) VALUES (?, ?)', [name, score]);
  }

  async getTopScores(limit: number): Promise<LeaderboardEntry[]> {
    return this.db.query('SELECT id, name, score, timestamp FROM scores ORDER BY score DESC LIMIT ?').all(limit) as LeaderboardEntry[];
  }
}
