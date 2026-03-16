/**
 * HTTP API — Primary Adapter
 * Exposes leaderboard endpoints using Bun.serve.
 */

import type { ILeaderboardPort } from '../../core/ports/index.js';

export class HttpApi {
  constructor(private readonly leaderboard: ILeaderboardPort) {}

  handler(): (req: Request) => Promise<Response> {
    return async (req: Request): Promise<Response> => {
      const url = new URL(req.url);

      if (url.pathname === '/api/scores' && req.method === 'GET') {
        const scores = await this.leaderboard.getTopScores(10);
        return Response.json(scores, {
          headers: { 'Access-Control-Allow-Origin': '*' },
        });
      }

      if (url.pathname === '/api/scores' && req.method === 'POST') {
        const body = await req.json() as { name: string; score: number };
        if (!body.name || typeof body.score !== 'number') {
          return Response.json({ error: 'name and score required' }, { status: 400 });
        }
        const sanitizedName = body.name.slice(0, 20).replace(/[<>&"']/g, '');
        await this.leaderboard.saveScore(sanitizedName, body.score);
        return Response.json({ ok: true }, {
          headers: { 'Access-Control-Allow-Origin': '*' },
        });
      }

      if (req.method === 'OPTIONS') {
        return new Response(null, {
          headers: {
            'Access-Control-Allow-Origin': '*',
            'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
            'Access-Control-Allow-Headers': 'Content-Type',
          },
        });
      }

      return Response.json({ error: 'not found' }, { status: 404 });
    };
  }
}
