import { describe, it, expect, vi, beforeEach } from 'vitest';
import { toasts, addToast, removeToast } from '../toast';

describe('toast store', () => {
  beforeEach(() => {
    // Clear all toasts by removing each one
    for (const t of toasts()) {
      removeToast(t.id);
    }
    vi.useFakeTimers();
  });

  it('addToast creates a toast with correct type and message', () => {
    addToast('success', 'Operation completed');
    const list = toasts();
    expect(list).toHaveLength(1);
    expect(list[0].type).toBe('success');
    expect(list[0].message).toBe('Operation completed');
    expect(list[0].id).toBeDefined();
  });

  it('addToast supports all toast types', () => {
    addToast('success', 'ok');
    addToast('error', 'fail');
    addToast('info', 'note');
    const types = toasts().map((t) => t.type);
    expect(types).toEqual(['success', 'error', 'info']);
  });

  it('removeToast removes a toast by id', () => {
    addToast('info', 'first');
    addToast('info', 'second');
    const id = toasts()[0].id;
    removeToast(id);
    expect(toasts()).toHaveLength(1);
    expect(toasts()[0].message).toBe('second');
  });

  it('multiple toasts can coexist', () => {
    addToast('success', 'a');
    addToast('error', 'b');
    addToast('info', 'c');
    expect(toasts()).toHaveLength(3);
  });

  it('auto-dismisses after durationMs', () => {
    addToast('info', 'temporary', 2000);
    expect(toasts()).toHaveLength(1);
    vi.advanceTimersByTime(2000);
    expect(toasts()).toHaveLength(0);
  });
});
