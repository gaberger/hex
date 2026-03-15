/**
 * Browser Input — Primary Adapter
 * Implements IInputPort via click, touch, and keyboard events.
 */

import type { IInputPort } from '../../core/ports/index.js';

export class BrowserInput implements IInputPort {
  private callback: (() => void) | null = null;
  private readonly handleClick: () => void;
  private readonly handleKeydown: (e: KeyboardEvent) => void;
  private readonly handleTouchstart: (e: TouchEvent) => void;

  constructor(private readonly element: HTMLElement) {
    this.handleClick = () => this.callback?.();
    this.handleKeydown = (e: KeyboardEvent) => {
      if (e.code === 'Space' || e.key === ' ') {
        e.preventDefault();
        this.callback?.();
      }
    };
    this.handleTouchstart = (e: TouchEvent) => {
      e.preventDefault();
      this.callback?.();
    };
  }

  onAction(callback: () => void): void {
    this.callback = callback;
    this.element.addEventListener('click', this.handleClick);
    document.addEventListener('keydown', this.handleKeydown);
    this.element.addEventListener('touchstart', this.handleTouchstart, { passive: false });
  }

  destroy(): void {
    this.element.removeEventListener('click', this.handleClick);
    document.removeEventListener('keydown', this.handleKeydown);
    this.element.removeEventListener('touchstart', this.handleTouchstart);
    this.callback = null;
  }
}
