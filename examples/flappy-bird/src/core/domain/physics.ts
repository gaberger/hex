import type { Vec2, Rect, PipeState, GameConfig } from '../ports/index.js';

const BIRD_SIZE = 20;

export function applyGravity(velocity: number, gravity: number, deltaMs: number): number {
  return velocity + gravity * (deltaMs / 1000);
}

export function applyFlap(velocity: number, flapStrength: number): number {
  return -flapStrength;
}

export function updatePosition(pos: Vec2, velocity: number, deltaMs: number): Vec2 {
  return {
    x: pos.x,
    y: pos.y + velocity * (deltaMs / 1000),
  };
}

export function calculateRotation(velocity: number): number {
  const maxAngle = Math.PI / 4;
  const clampedVel = Math.max(-400, Math.min(400, velocity));
  return (clampedVel / 400) * maxAngle;
}

export function checkCollision(bird: Rect, pipe: PipeState, config: GameConfig): boolean {
  const halfGap = config.pipeGap / 2;
  const topPipeBottom = pipe.gapY - halfGap;
  const bottomPipeTop = pipe.gapY + halfGap;

  const birdRight = bird.x + bird.width;
  const birdBottom = bird.y + bird.height;
  const pipeRight = pipe.x + config.pipeWidth;

  if (birdRight <= pipe.x || bird.x >= pipeRight) {
    return false;
  }

  return bird.y <= topPipeBottom || birdBottom >= bottomPipeTop;
}

export function checkBounds(bird: Vec2, canvasHeight: number): boolean {
  return bird.y <= 0 || bird.y + BIRD_SIZE >= canvasHeight;
}

export function generatePipeGapY(canvasHeight: number, pipeGap: number): number {
  const margin = pipeGap / 2 + 40;
  const min = margin;
  const max = canvasHeight - margin;
  return min + Math.random() * (max - min);
}

export function shouldSpawnPipe(tick: number, interval: number): boolean {
  return tick > 0 && tick % interval === 0;
}

export function isPastPipe(birdX: number, pipeX: number, pipeWidth: number): boolean {
  return birdX > pipeX + pipeWidth;
}

export function getBirdRect(pos: Vec2): Rect {
  return { x: pos.x, y: pos.y, width: BIRD_SIZE, height: BIRD_SIZE };
}
