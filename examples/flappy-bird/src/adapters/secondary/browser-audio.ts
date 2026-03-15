/**
 * Browser Audio — Secondary Adapter
 * Implements IAudioPort using Web Audio API oscillators.
 */

import type { IAudioPort } from '../../core/ports/index.js';

export class BrowserAudio implements IAudioPort {
  private ctx: AudioContext | null = null;

  private getContext(): AudioContext {
    if (!this.ctx) {
      this.ctx = new AudioContext();
    }
    return this.ctx;
  }

  playFlap(): void {
    this.playTone(440, 0.08, 'sine');
  }

  playScore(): void {
    this.playTone(880, 0.1, 'sine');
  }

  playHit(): void {
    this.playTone(200, 0.2, 'sawtooth');
  }

  private playTone(frequency: number, duration: number, type: OscillatorType): void {
    try {
      const ctx = this.getContext();
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();

      osc.type = type;
      osc.frequency.setValueAtTime(frequency, ctx.currentTime);
      gain.gain.setValueAtTime(0.15, ctx.currentTime);
      gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + duration);

      osc.connect(gain);
      gain.connect(ctx.destination);
      osc.start(ctx.currentTime);
      osc.stop(ctx.currentTime + duration);
    } catch {
      // Silently ignore audio errors (e.g., autoplay policy)
    }
  }
}
