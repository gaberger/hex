/**
 * Leaderboard Display — Primary Adapter
 * Shows leaderboard scores on the game-over screen and handles score submission.
 * Uses DOM APIs only (no innerHTML) per security rules.
 */

import type { LeaderboardEntry } from '../../core/ports/index.js';

export class LeaderboardDisplay {
  private readonly apiBase: string;
  private cachedScores: LeaderboardEntry[] = [];
  private submitted = false;

  constructor(apiBase: string = 'http://localhost:3001') {
    this.apiBase = apiBase;
  }

  /**
   * Fetch top scores from the HTTP API.
   */
  async fetchScores(limit: number = 5): Promise<LeaderboardEntry[]> {
    try {
      const res = await fetch(`${this.apiBase}/api/scores`);
      if (!res.ok) return this.cachedScores;
      const scores = (await res.json()) as LeaderboardEntry[];
      this.cachedScores = scores.slice(0, limit);
      return this.cachedScores;
    } catch {
      return this.cachedScores;
    }
  }

  /**
   * Submit a score to the HTTP API.
   */
  async submitScore(name: string, score: number): Promise<boolean> {
    try {
      const res = await fetch(`${this.apiBase}/api/scores`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name, score }),
      });
      if (res.ok) {
        this.submitted = true;
        await this.fetchScores();
        return true;
      }
      return false;
    } catch {
      return false;
    }
  }

  /**
   * Draw the leaderboard overlay on a canvas context.
   * Called during game-over phase rendering.
   */
  drawLeaderboard(ctx: CanvasRenderingContext2D, canvasWidth: number, startY: number): void {
    const scores = this.cachedScores;
    if (scores.length === 0) return;

    const centerX = canvasWidth / 2;

    // Title
    ctx.font = 'bold 16px Arial';
    ctx.textAlign = 'center';
    ctx.fillStyle = '#fff';
    ctx.strokeStyle = '#000';
    ctx.lineWidth = 2;
    ctx.strokeText('LEADERBOARD', centerX, startY);
    ctx.fillText('LEADERBOARD', centerX, startY);

    // Score entries
    ctx.font = '14px Arial';
    ctx.lineWidth = 1.5;
    for (let i = 0; i < scores.length; i++) {
      const entry = scores[i];
      const y = startY + 22 + i * 20;
      const text = `${i + 1}. ${entry.name} — ${entry.score}`;
      ctx.strokeText(text, centerX, y);
      ctx.fillText(text, centerX, y);
    }
  }

  /**
   * Prompt the player for their name and submit the score.
   * Returns true if submitted, false if cancelled or already submitted.
   */
  async promptAndSubmit(score: number): Promise<boolean> {
    if (this.submitted) return false;
    const name = prompt('Enter your name for the leaderboard:');
    if (!name || name.trim().length === 0) return false;
    return this.submitScore(name.trim(), score);
  }

  /**
   * Reset submission state for a new game.
   */
  resetSubmission(): void {
    this.submitted = false;
  }

  /**
   * Whether a score has already been submitted this round.
   */
  hasSubmitted(): boolean {
    return this.submitted;
  }
}
