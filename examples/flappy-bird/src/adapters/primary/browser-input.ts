import type { IInputPort } from '../../core/ports/index.js';

export class BrowserInput implements IInputPort {
  private callback: (() => void) | null = null;
  private clickHandler: ((e: Event) => void) | null = null;
  private keyHandler: ((e: Event) => void) | null = null;

  onFlap(callback: () => void): void {
    this.callback = callback;
  }

  start(): void {
    this.clickHandler = (e: Event) => {
      e.preventDefault();
      this.callback?.();
    };
    this.keyHandler = (e: Event) => {
      if ((e as KeyboardEvent).code === 'Space') {
        e.preventDefault();
        this.callback?.();
      }
    };
    document.addEventListener('click', this.clickHandler);
    document.addEventListener('touchstart', this.clickHandler, { passive: false });
    document.addEventListener('keydown', this.keyHandler);
  }

  stop(): void {
    if (this.clickHandler) {
      document.removeEventListener('click', this.clickHandler);
      document.removeEventListener('touchstart', this.clickHandler);
    }
    if (this.keyHandler) {
      document.removeEventListener('keydown', this.keyHandler);
    }
    this.clickHandler = null;
    this.keyHandler = null;
  }
}
