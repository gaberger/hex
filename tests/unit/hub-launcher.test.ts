import { describe, it, expect, mock, beforeEach } from 'bun:test';

// Mock node:fs and node:child_process before importing
const mockExistsSync = mock(() => false);
const mockReadFileSync = mock(() => '{}');
const mockSpawn = mock(() => ({ unref: () => {}, pid: 12345 }));

mock.module('node:fs', () => ({
  existsSync: mockExistsSync,
  readFileSync: mockReadFileSync,
}));

mock.module('node:child_process', () => ({
  spawn: mockSpawn,
}));

// Import after mocks are set up
const { HubLauncher, ensureHubRunning } = await import(
  '../../src/adapters/secondary/hub-launcher.js'
);

describe('HubLauncher', () => {
  let launcher: InstanceType<typeof HubLauncher>;

  beforeEach(() => {
    launcher = new HubLauncher();
    mockExistsSync.mockReset();
    mockReadFileSync.mockReset();
    mockSpawn.mockReset();
  });

  describe('findBinary', () => {
    it('returns null when no binary exists', () => {
      mockExistsSync.mockReturnValue(false);
      expect(launcher.findBinary()).toBeNull();
    });

    it('returns the first existing path', () => {
      // First call returns false, second returns true
      let callCount = 0;
      mockExistsSync.mockImplementation(() => {
        callCount++;
        return callCount === 2;
      });
      const result = launcher.findBinary();
      expect(result).not.toBeNull();
      expect(result).toContain('hex-hub');
    });
  });

  describe('isRunning', () => {
    it('returns a boolean', async () => {
      const result = await launcher.isRunning();
      expect(typeof result).toBe('boolean');
    });
  });

  describe('status', () => {
    it('returns an object with running, url, and projects fields', async () => {
      const result = await launcher.status();
      expect(typeof result.running).toBe('boolean');
      expect(typeof result.projects).toBe('number');
      if (result.running) {
        expect(result.url).toContain('127.0.0.1');
      } else {
        expect(result.url).toBeNull();
        expect(result.projects).toBe(0);
      }
    });
  });

  describe('stop', () => {
    it('returns false when no lock file exists', async () => {
      mockReadFileSync.mockImplementation(() => {
        throw new Error('ENOENT');
      });
      const result = await launcher.stop();
      expect(result).toBe(false);
    });
  });
});

describe('ensureHubRunning', () => {
  it('is exported as a function', () => {
    expect(typeof ensureHubRunning).toBe('function');
  });
});
