/**
 * Leaderboard Server — Composition Root
 * Wires SQLite adapter to HTTP API adapter and starts the server.
 */

import { SqliteLeaderboard } from './adapters/secondary/sqlite-leaderboard.js';
import { HttpApi } from './adapters/primary/http-api.js';

const PORT = Number(process.env.PORT) || 3001;

const leaderboard = new SqliteLeaderboard('leaderboard.db');
const api = new HttpApi(leaderboard);

Bun.serve({
  port: PORT,
  fetch: api.handler(),
});

console.log(`Leaderboard API running on http://localhost:${PORT}`);
