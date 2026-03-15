import type { IAudioPort } from '../../core/ports/index.js';

export class BrowserAudio implements IAudioPort {
  private ctx: AudioContext | null = null;

  private ensureContext(): AudioContext {
    if (!this.ctx) this.ctx = new AudioContext();
    return this.ctx;
  }

  private beep(frequency: number, durationMs: number): void {
    try {
      const ctx = this.ensureContext();
      const osc = ctx.createOscillator();
      const gain = ctx.createGain();
      osc.type = 'square';
      osc.frequency.value = frequency;
      gain.gain.value = 0.1;
      osc.connect(gain).connect(ctx.destination);
      osc.start();
      osc.stop(ctx.currentTime + durationMs / 1000);
    } catch { /* audio may be blocked by browser policy */ }
  }

  playFlap(): void { this.beep(400, 50); }
  playScore(): void { this.beep(600, 100); }
  playHit(): void { this.beep(200, 200); }
}
