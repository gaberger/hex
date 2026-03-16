import { describe, it, expect, beforeEach, mock } from 'bun:test';
import { FileCheckpointAdapter } from '../../src/adapters/secondary/file-checkpoint-adapter.js';
import type { IFileSystemPort } from '../../src/core/ports/index.js';
import type { CheckpointEntry, TaskSnapshot } from '../../src/core/domain/checkpoint-types.js';

// ─── Factory Helpers ────────────────────────────────────────

function makeTaskSnapshot(overrides?: Partial<TaskSnapshot>): TaskSnapshot {
  return {
    taskId: 'task-1',
    title: 'feat/auth/implement login',
    status: 'completed',
    agentRole: 'coder',
    assignee: 'agent-1',
    snapshotAt: '2025-06-01T00:00:00.000Z',
    ...overrides,
  };
}

function makeCheckpointEntry(overrides?: Partial<CheckpointEntry>): CheckpointEntry {
  return {
    id: 'ckpt-1',
    projectId: 'test-project',
    projectPath: '/tmp/test-project',
    createdAt: '2025-06-01T12:00:00.000Z',
    swarmStatus: { topology: 'hierarchical', agentCount: 3, status: 'running' },
    features: [],
    orphanTasks: [makeTaskSnapshot()],
    ...overrides,
  };
}

// ─── Mock FS Builder ────────────────────────────────────────

function makeMockFs(): IFileSystemPort {
  return {
    read: mock(() => Promise.resolve('')),
    write: mock(() => Promise.resolve()),
    exists: mock(() => Promise.resolve(true)),
    glob: mock(() => Promise.resolve([])),
  };
}

// ─── Tests ──────────────────────────────────────────────────

describe('FileCheckpointAdapter', () => {
  let fs: IFileSystemPort;
  let adapter: FileCheckpointAdapter;
  const basePath = '/data/checkpoints';

  beforeEach(() => {
    fs = makeMockFs();
    adapter = new FileCheckpointAdapter(basePath, fs);
  });

  describe('checkpoint', () => {
    it('writes valid JSON to the expected path', async () => {
      const entry = makeCheckpointEntry({ createdAt: '2025-06-01T12:00:00.000Z' });

      await adapter.checkpoint(entry);

      // Should write to tmp then to final path (atomic write pattern)
      expect(fs.write).toHaveBeenCalledTimes(3); // tmp, final, clear tmp
      const calls = (fs.write as ReturnType<typeof mock>).mock.calls;

      // Second call is the final write
      const finalPath = calls[1][0] as string;
      const finalContent = calls[1][1] as string;

      expect(finalPath).toBe('/data/checkpoints/test-project/2025-06-01T12-00-00.000Z.json');
      const parsed = JSON.parse(finalContent);
      expect(parsed.id).toBe('ckpt-1');
      expect(parsed.projectId).toBe('test-project');
    });

    it('does not throw on write failure', async () => {
      (fs.write as ReturnType<typeof mock>).mockImplementation(() => Promise.reject(new Error('disk full')));

      const entry = makeCheckpointEntry();

      // Should not throw — port contract says checkpoint must not throw on write failure
      await adapter.checkpoint(entry);
    });

    it('sanitises project IDs with unsafe characters', async () => {
      const entry = makeCheckpointEntry({ projectId: 'my/project:name' });

      await adapter.checkpoint(entry);

      const calls = (fs.write as ReturnType<typeof mock>).mock.calls;
      const finalPath = calls[1][0] as string;
      expect(finalPath).toContain('my_project_name');
      expect(finalPath).not.toContain('/my/');
    });
  });

  describe('recover', () => {
    it('returns the most recent checkpoint by timestamp', async () => {
      const older = makeCheckpointEntry({ id: 'old', createdAt: '2025-06-01T10:00:00.000Z' });
      const newer = makeCheckpointEntry({ id: 'new', createdAt: '2025-06-01T14:00:00.000Z' });

      (fs.glob as ReturnType<typeof mock>).mockReturnValue(Promise.resolve([
        '/data/checkpoints/test-project/2025-06-01T10-00-00.000Z.json',
        '/data/checkpoints/test-project/2025-06-01T14-00-00.000Z.json',
      ]));
      (fs.read as ReturnType<typeof mock>).mockImplementation((path: string) => {
        if (path.includes('14-00')) return Promise.resolve(JSON.stringify(newer));
        return Promise.resolve(JSON.stringify(older));
      });

      const result = await adapter.recover('test-project');

      expect(result).not.toBeNull();
      expect(result!.id).toBe('new');
    });

    it('returns null when no checkpoints exist', async () => {
      (fs.exists as ReturnType<typeof mock>).mockReturnValue(Promise.resolve(false));

      const result = await adapter.recover('test-project');

      expect(result).toBeNull();
    });

    it('returns null when directory exists but has no files', async () => {
      (fs.exists as ReturnType<typeof mock>).mockReturnValue(Promise.resolve(true));
      (fs.glob as ReturnType<typeof mock>).mockReturnValue(Promise.resolve([]));

      const result = await adapter.recover('test-project');

      expect(result).toBeNull();
    });
  });

  describe('list', () => {
    it('returns all checkpoints sorted newest-first', async () => {
      const e1 = makeCheckpointEntry({ id: 'first', createdAt: '2025-06-01T08:00:00.000Z' });
      const e2 = makeCheckpointEntry({ id: 'second', createdAt: '2025-06-01T12:00:00.000Z' });
      const e3 = makeCheckpointEntry({ id: 'third', createdAt: '2025-06-01T16:00:00.000Z' });

      (fs.glob as ReturnType<typeof mock>).mockReturnValue(Promise.resolve([
        '/data/checkpoints/test-project/2025-06-01T08-00-00.000Z.json',
        '/data/checkpoints/test-project/2025-06-01T12-00-00.000Z.json',
        '/data/checkpoints/test-project/2025-06-01T16-00-00.000Z.json',
      ]));
      (fs.read as ReturnType<typeof mock>).mockImplementation((path: string) => {
        if (path.includes('08-00')) return Promise.resolve(JSON.stringify(e1));
        if (path.includes('12-00')) return Promise.resolve(JSON.stringify(e2));
        return Promise.resolve(JSON.stringify(e3));
      });

      const entries = await adapter.list('test-project');

      expect(entries.length).toBe(3);
      expect(entries[0].id).toBe('third');   // newest first
      expect(entries[1].id).toBe('second');
      expect(entries[2].id).toBe('first');
    });

    it('skips empty (pruned) files', async () => {
      const valid = makeCheckpointEntry({ id: 'valid' });

      (fs.glob as ReturnType<typeof mock>).mockReturnValue(Promise.resolve([
        '/data/checkpoints/test-project/2025-06-01T08-00-00.000Z.json',
        '/data/checkpoints/test-project/2025-06-01T12-00-00.000Z.json',
      ]));
      (fs.read as ReturnType<typeof mock>).mockImplementation((path: string) => {
        if (path.includes('08-00')) return Promise.resolve(''); // pruned
        return Promise.resolve(JSON.stringify(valid));
      });

      const entries = await adapter.list('test-project');

      expect(entries.length).toBe(1);
      expect(entries[0].id).toBe('valid');
    });
  });

  describe('prune', () => {
    it('keeps exactly N checkpoints and removes the rest', async () => {
      (fs.glob as ReturnType<typeof mock>).mockReturnValue(Promise.resolve([
        '/data/checkpoints/test-project/2025-06-01T08-00-00.000Z.json',
        '/data/checkpoints/test-project/2025-06-01T10-00-00.000Z.json',
        '/data/checkpoints/test-project/2025-06-01T12-00-00.000Z.json',
        '/data/checkpoints/test-project/2025-06-01T14-00-00.000Z.json',
        '/data/checkpoints/test-project/2025-06-01T16-00-00.000Z.json',
      ]));

      const deleted = await adapter.prune('test-project', 3);

      expect(deleted).toBe(2);
      // Should have written empty content to the two oldest files
      const writeCalls = (fs.write as ReturnType<typeof mock>).mock.calls;
      expect(writeCalls.length).toBe(2);
      expect(writeCalls[0][0]).toContain('08-00');
      expect(writeCalls[0][1]).toBe('');
      expect(writeCalls[1][0]).toContain('10-00');
      expect(writeCalls[1][1]).toBe('');
    });

    it('returns 0 when fewer checkpoints than keepCount', async () => {
      (fs.glob as ReturnType<typeof mock>).mockReturnValue(Promise.resolve([
        '/data/checkpoints/test-project/2025-06-01T08-00-00.000Z.json',
      ]));

      const deleted = await adapter.prune('test-project', 3);

      expect(deleted).toBe(0);
      expect(fs.write).not.toHaveBeenCalled();
    });
  });

  describe('round-trip', () => {
    it('checkpoint then recover returns equivalent data', async () => {
      const original = makeCheckpointEntry({
        id: 'round-trip-1',
        createdAt: '2025-06-01T12:00:00.000Z',
        features: [{
          featureId: 'auth',
          title: 'auth',
          phase: 'code',
          totalSteps: 3,
          completedSteps: 1,
          failedSteps: 0,
          startedAt: '2025-06-01T10:00:00.000Z',
          updatedAt: '2025-06-01T12:00:00.000Z',
          taskSnapshots: [makeTaskSnapshot()],
        }],
        orphanTasks: [makeTaskSnapshot({ taskId: 'orphan-1', title: 'cleanup' })],
      });

      // Capture what checkpoint writes
      let writtenJson = '';
      (fs.write as ReturnType<typeof mock>).mockImplementation((_path: string, content: string) => {
        if (content.length > 0) writtenJson = content;
        return Promise.resolve();
      });

      await adapter.checkpoint(original);

      // Set up recover to return the written data
      (fs.glob as ReturnType<typeof mock>).mockReturnValue(Promise.resolve([
        '/data/checkpoints/test-project/2025-06-01T12-00-00.000Z.json',
      ]));
      (fs.read as ReturnType<typeof mock>).mockReturnValue(Promise.resolve(writtenJson));

      const recovered = await adapter.recover('test-project');

      expect(recovered).not.toBeNull();
      expect(recovered!.id).toBe(original.id);
      expect(recovered!.projectId).toBe(original.projectId);
      expect(recovered!.features.length).toBe(1);
      expect(recovered!.features[0].featureId).toBe('auth');
      expect(recovered!.orphanTasks.length).toBe(1);
      expect(recovered!.orphanTasks[0].taskId).toBe('orphan-1');
    });
  });
});
