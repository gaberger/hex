import { describe, it, expect } from 'bun:test';
import { ProgressReporter } from '../../src/adapters/primary/progress-reporter.js';

function createCapture(): { lines: string[]; write: (text: string) => void } {
  const lines: string[] = [];
  return { lines, write: (text: string) => lines.push(text) };
}

describe('ProgressReporter', () => {
  describe('phase()', () => {
    it('outputs tree-style prefix with name', () => {
      const { lines, write } = createCapture();
      const reporter = new ProgressReporter(write);

      reporter.phase('Scanning');

      expect(lines).toHaveLength(1);
      expect(lines[0]).toContain('\u251C\u2500'); // ├─
      expect(lines[0]).toContain('Scanning');
    });

    it('includes detail when provided', () => {
      const { lines, write } = createCapture();
      const reporter = new ProgressReporter(write);

      reporter.phase('Config', 'loaded .hexrc');

      expect(lines[0]).toContain('Config');
      expect(lines[0]).toContain('loaded .hexrc');
    });
  });

  describe('phaseFinal()', () => {
    it('uses the final tree prefix', () => {
      const { lines, write } = createCapture();
      const reporter = new ProgressReporter(write);

      reporter.phaseFinal('Done');

      expect(lines[0]).toContain('\u2514\u2500'); // └─
      expect(lines[0]).toContain('Done');
    });
  });

  describe('scanning()', () => {
    it('shows file count', () => {
      const { lines, write } = createCapture();
      const reporter = new ProgressReporter(write);

      reporter.scanning(1500, 0, 200);

      expect(lines).toHaveLength(1);
      expect(lines[0]).toContain('1,500');
      expect(lines[0]).toContain('files found');
    });

    it('shows excluded count when non-zero', () => {
      const { lines, write } = createCapture();
      const reporter = new ProgressReporter(write);

      reporter.scanning(1500, 300, 200);

      expect(lines).toHaveLength(2);
      expect(lines[1]).toContain('300');
      expect(lines[1]).toContain('excluded');
    });

    it('does not show excluded line when zero', () => {
      const { lines, write } = createCapture();
      const reporter = new ProgressReporter(write);

      reporter.scanning(500, 0, 100);

      expect(lines).toHaveLength(1);
    });
  });

  describe('indexing()', () => {
    it('shows progress bar with percentage', () => {
      const { lines, write } = createCapture();
      const reporter = new ProgressReporter(write);

      reporter.indexing(50, 100, 500);

      // Should produce two lines: phase line + subPhase with bar
      expect(lines).toHaveLength(2);
      expect(lines[1]).toContain('50%');
      expect(lines[1]).toContain('50/100');
      // Should contain block characters for the bar
      expect(lines[1]).toContain('\u2588'); // filled block
    });

    it('shows 0% when no progress', () => {
      const { lines, write } = createCapture();
      const reporter = new ProgressReporter(write);

      reporter.indexing(0, 100, 0);

      expect(lines[1]).toContain('0%');
    });

    it('shows 100% when complete', () => {
      const { lines, write } = createCapture();
      const reporter = new ProgressReporter(write);

      reporter.indexing(200, 200, 3000);

      expect(lines[1]).toContain('100%');
      // Duration should be formatted as seconds
      expect(lines[1]).toContain('3.0s');
    });
  });

  describe('complete()', () => {
    it('shows summary with file count and duration', () => {
      const { lines, write } = createCapture();
      const reporter = new ProgressReporter(write);

      reporter.complete({ files: 2500, excluded: 100, duration: 1500 });

      expect(lines).toHaveLength(1);
      expect(lines[0]).toContain('\u2514\u2500'); // └─ (final prefix)
      expect(lines[0]).toContain('2,500');
      expect(lines[0]).toContain('1.5s');
      expect(lines[0]).toContain('Complete');
    });

    it('shows milliseconds for short durations', () => {
      const { lines, write } = createCapture();
      const reporter = new ProgressReporter(write);

      reporter.complete({ files: 50, excluded: 0, duration: 42 });

      expect(lines[0]).toContain('42ms');
    });
  });

  describe('header()', () => {
    it('outputs the message directly', () => {
      const { lines, write } = createCapture();
      const reporter = new ProgressReporter(write);

      reporter.header('hex init v1.0');

      expect(lines).toEqual(['hex init v1.0']);
    });
  });

  describe('subPhase()', () => {
    it('uses nested tree prefix', () => {
      const { lines, write } = createCapture();
      const reporter = new ProgressReporter(write);

      reporter.subPhase('detail', 'extra info');

      expect(lines[0]).toContain('\u2502'); // │
      expect(lines[0]).toContain('\u2514\u2500'); // └─
      expect(lines[0]).toContain('detail');
      expect(lines[0]).toContain('extra info');
    });
  });
});
