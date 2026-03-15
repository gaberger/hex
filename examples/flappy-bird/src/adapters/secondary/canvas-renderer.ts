/**
 * Canvas Renderer — Secondary Adapter
 * Implements IRenderPort using HTML5 Canvas 2D.
 */

import type { IRenderPort, Bird, Pipe, Phase, GameConfig } from '../../core/ports/index.js';

export class CanvasRenderer implements IRenderPort {
  private readonly ctx: CanvasRenderingContext2D;

  constructor(private readonly canvas: HTMLCanvasElement) {
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('Canvas 2D context not available');
    this.ctx = ctx;
  }

  clear(): void {
    // Sky gradient
    const grad = this.ctx.createLinearGradient(0, 0, 0, this.canvas.height);
    grad.addColorStop(0, '#4dc9f6');
    grad.addColorStop(1, '#a7e8f0');
    this.ctx.fillStyle = grad;
    this.ctx.fillRect(0, 0, this.canvas.width, this.canvas.height);
  }

  drawBird(bird: Bird): void {
    this.ctx.save();
    this.ctx.translate(bird.x + 12, bird.y + 12);
    this.ctx.rotate(bird.rotation);

    // Body
    this.ctx.fillStyle = '#f5c542';
    this.ctx.beginPath();
    this.ctx.ellipse(0, 0, 14, 10, 0, 0, Math.PI * 2);
    this.ctx.fill();
    this.ctx.strokeStyle = '#d4a017';
    this.ctx.lineWidth = 1.5;
    this.ctx.stroke();

    // Eye
    this.ctx.fillStyle = '#fff';
    this.ctx.beginPath();
    this.ctx.arc(6, -3, 4, 0, Math.PI * 2);
    this.ctx.fill();
    this.ctx.fillStyle = '#000';
    this.ctx.beginPath();
    this.ctx.arc(7, -3, 2, 0, Math.PI * 2);
    this.ctx.fill();

    // Beak
    this.ctx.fillStyle = '#e87d2f';
    this.ctx.beginPath();
    this.ctx.moveTo(12, -1);
    this.ctx.lineTo(18, 2);
    this.ctx.lineTo(12, 5);
    this.ctx.closePath();
    this.ctx.fill();

    // Wing
    this.ctx.fillStyle = '#e8b732';
    this.ctx.beginPath();
    this.ctx.ellipse(-4, 3, 8, 5, -0.3, 0, Math.PI * 2);
    this.ctx.fill();

    this.ctx.restore();
  }

  drawPipe(pipe: Pipe, config: GameConfig): void {
    const { pipeWidth, pipeGap } = config;
    const gapTop = pipe.gapY;
    const gapBottom = pipe.gapY + pipeGap;

    // Top pipe
    this.ctx.fillStyle = '#73bf2e';
    this.ctx.fillRect(pipe.x, 0, pipeWidth, gapTop);
    this.ctx.fillStyle = '#5a9e1e';
    this.ctx.fillRect(pipe.x - 3, gapTop - 24, pipeWidth + 6, 24);

    // Bottom pipe
    this.ctx.fillStyle = '#73bf2e';
    this.ctx.fillRect(pipe.x, gapBottom, pipeWidth, this.canvas.height - gapBottom);
    this.ctx.fillStyle = '#5a9e1e';
    this.ctx.fillRect(pipe.x - 3, gapBottom, pipeWidth + 6, 24);
  }

  drawGround(config: GameConfig): void {
    const groundY = config.canvasHeight - config.groundHeight;
    this.ctx.fillStyle = '#deb887';
    this.ctx.fillRect(0, groundY, config.canvasWidth, config.groundHeight);
    this.ctx.fillStyle = '#8fce00';
    this.ctx.fillRect(0, groundY, config.canvasWidth, 4);
  }

  drawScore(score: number): void {
    this.ctx.fillStyle = '#fff';
    this.ctx.strokeStyle = '#000';
    this.ctx.lineWidth = 3;
    this.ctx.font = 'bold 36px Arial';
    this.ctx.textAlign = 'center';
    const text = String(score);
    this.ctx.strokeText(text, this.canvas.width / 2, 50);
    this.ctx.fillText(text, this.canvas.width / 2, 50);
  }

  drawOverlay(phase: Phase, score: number, highScore: number): void {
    if (phase === 'ready') {
      this.drawCenteredText('TAP TO START', this.canvas.height / 2);
    } else if (phase === 'gameover') {
      this.drawCenteredText('GAME OVER', this.canvas.height / 2 - 40);
      this.ctx.font = 'bold 20px Arial';
      this.ctx.fillStyle = '#fff';
      this.ctx.strokeStyle = '#000';
      this.ctx.lineWidth = 2;
      this.ctx.strokeText(`Score: ${score}`, this.canvas.width / 2, this.canvas.height / 2);
      this.ctx.fillText(`Score: ${score}`, this.canvas.width / 2, this.canvas.height / 2);
      this.ctx.strokeText(`Best: ${highScore}`, this.canvas.width / 2, this.canvas.height / 2 + 30);
      this.ctx.fillText(`Best: ${highScore}`, this.canvas.width / 2, this.canvas.height / 2 + 30);
      this.drawCenteredText('TAP TO RESTART', this.canvas.height / 2 + 70);
    }
  }

  private drawCenteredText(text: string, y: number): void {
    this.ctx.font = 'bold 24px Arial';
    this.ctx.textAlign = 'center';
    this.ctx.fillStyle = '#fff';
    this.ctx.strokeStyle = '#000';
    this.ctx.lineWidth = 3;
    this.ctx.strokeText(text, this.canvas.width / 2, y);
    this.ctx.fillText(text, this.canvas.width / 2, y);
  }
}
