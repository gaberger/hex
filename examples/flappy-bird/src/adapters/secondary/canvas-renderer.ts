import type { IRenderPort, GameState, GameConfig, BirdState, PipeState } from '../../core/ports/index.js';

const BIRD_SIZE = 20;

export class CanvasRenderer implements IRenderPort {
  private ctx: CanvasRenderingContext2D | null = null;
  private canvas: HTMLCanvasElement | null = null;

  async init(config: GameConfig): Promise<void> {
    this.canvas = document.getElementById('game-canvas') as HTMLCanvasElement
      ?? document.createElement('canvas');
    this.canvas.id = 'game-canvas';
    this.canvas.width = config.canvasWidth;
    this.canvas.height = config.canvasHeight;
    if (!this.canvas.parentElement) document.body.appendChild(this.canvas);
    this.ctx = this.canvas.getContext('2d')!;
  }

  render(state: GameState, config: GameConfig): void {
    const ctx = this.ctx!;
    const { canvasWidth: w, canvasHeight: h } = config;

    // Background — sky gradient
    const bg = ctx.createLinearGradient(0, 0, 0, h);
    bg.addColorStop(0, '#87CEEB');
    bg.addColorStop(1, '#E0F0FF');
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, w, h);

    // Ground line
    ctx.fillStyle = '#8B5E3C';
    ctx.fillRect(0, h - 20, w, 20);
    ctx.fillStyle = '#4CAF50';
    ctx.fillRect(0, h - 20, w, 4);

    // Pipes
    this.drawPipes(ctx, state.pipes, config);

    // Bird
    this.drawBird(ctx, state.bird);

    // Score
    this.drawScore(ctx, state.score, w);

    // Overlays
    if (state.phase === 'ready') {
      this.drawCenteredText(ctx, w, h, 'TAP TO START', 'rgba(0,0,0,0.3)');
    } else if (state.phase === 'gameover') {
      ctx.fillStyle = 'rgba(0,0,0,0.5)';
      ctx.fillRect(0, 0, w, h);
      this.drawCenteredText(ctx, w, h, 'GAME OVER', 'transparent');
      ctx.font = '20px sans-serif';
      ctx.fillStyle = '#FFF';
      ctx.textAlign = 'center';
      ctx.fillText('TAP TO RESTART', w / 2, h / 2 + 40);
      ctx.fillText(`High Score: ${state.highScore}`, w / 2, h / 2 + 70);
    }
  }

  destroy(): void {
    this.canvas?.remove();
    this.canvas = null;
    this.ctx = null;
  }

  // ── Private helpers ──

  private drawPipes(ctx: CanvasRenderingContext2D, pipes: PipeState[], config: GameConfig): void {
    const { pipeWidth, pipeGap, canvasHeight } = config;
    const halfGap = pipeGap / 2;

    for (const pipe of pipes) {
      const topH = pipe.gapY - halfGap;
      const bottomY = pipe.gapY + halfGap;

      // Top pipe
      ctx.fillStyle = '#388E3C';
      ctx.fillRect(pipe.x, 0, pipeWidth, topH);
      ctx.fillStyle = '#2E7D32';
      ctx.fillRect(pipe.x - 3, topH - 20, pipeWidth + 6, 20);

      // Bottom pipe
      ctx.fillStyle = '#388E3C';
      ctx.fillRect(pipe.x, bottomY, pipeWidth, canvasHeight - bottomY);
      ctx.fillStyle = '#2E7D32';
      ctx.fillRect(pipe.x - 3, bottomY, pipeWidth + 6, 20);
    }
  }

  private drawBird(ctx: CanvasRenderingContext2D, bird: BirdState): void {
    const cx = bird.position.x + BIRD_SIZE / 2;
    const cy = bird.position.y + BIRD_SIZE / 2;
    const r = BIRD_SIZE / 2;

    ctx.save();
    ctx.translate(cx, cy);
    ctx.rotate(bird.rotation);

    // Body
    ctx.fillStyle = '#FFD600';
    ctx.beginPath();
    ctx.arc(0, 0, r, 0, Math.PI * 2);
    ctx.fill();

    // Eye
    ctx.fillStyle = '#000';
    ctx.beginPath();
    ctx.arc(r * 0.35, -r * 0.2, r * 0.18, 0, Math.PI * 2);
    ctx.fill();

    // Beak
    ctx.fillStyle = '#FF6D00';
    ctx.beginPath();
    ctx.moveTo(r * 0.6, 0);
    ctx.lineTo(r * 1.3, -r * 0.15);
    ctx.lineTo(r * 1.3, r * 0.25);
    ctx.closePath();
    ctx.fill();

    ctx.restore();
  }

  private drawScore(ctx: CanvasRenderingContext2D, score: number, width: number): void {
    ctx.font = 'bold 48px sans-serif';
    ctx.textAlign = 'center';
    ctx.strokeStyle = '#000';
    ctx.lineWidth = 4;
    ctx.strokeText(String(score), width / 2, 60);
    ctx.fillStyle = '#FFF';
    ctx.fillText(String(score), width / 2, 60);
  }

  private drawCenteredText(
    ctx: CanvasRenderingContext2D, w: number, h: number,
    text: string, overlayColor: string,
  ): void {
    if (overlayColor !== 'transparent') {
      ctx.fillStyle = overlayColor;
      ctx.fillRect(0, 0, w, h);
    }
    ctx.font = 'bold 36px sans-serif';
    ctx.textAlign = 'center';
    ctx.strokeStyle = '#000';
    ctx.lineWidth = 3;
    ctx.strokeText(text, w / 2, h / 2);
    ctx.fillStyle = '#FFF';
    ctx.fillText(text, w / 2, h / 2);
  }
}
