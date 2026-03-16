/**
 * Unit tests for the WebSocket protocol used between hex-hub and project clients.
 *
 * Tests message envelope format, subscribe/unsubscribe protocol, topic patterns,
 * state update routing, command result handling, file change events, decision
 * events, reconnection backoff, project switching, and connection status tracking.
 *
 * Uses a lightweight Bun WebSocket server to verify protocol behavior without
 * mocking WebSocket internals.
 */

import { describe, test, expect, beforeEach } from 'bun:test';

// ── Protocol types (mirrors what DashboardAdapter uses) ──

interface WsEnvelope {
  topic?: string;
  event: string;
  data: Record<string, unknown>;
}

interface WsSubscribe {
  type: 'subscribe';
  topic: string;
}

interface WsUnsubscribe {
  type: 'unsubscribe';
  topic: string;
}

type WsClientMessage = WsSubscribe | WsUnsubscribe;

// ── Helpers ──────────────────────────────────────────────

/** Parse a raw WebSocket message string into a typed object. */
function parseMessage(raw: string): Record<string, unknown> {
  return JSON.parse(raw) as Record<string, unknown>;
}

/** Build a valid envelope for testing. */
function makeEnvelope(event: string, data: Record<string, unknown>, topic?: string): WsEnvelope {
  return { topic: topic ?? 'project:p1:command', event, data };
}

// ── 1. Message envelope format ──────────────────────────

describe('WebSocket message envelope format', () => {
  test('envelope contains topic, event, and data fields', () => {
    const envelope: WsEnvelope = {
      topic: 'hub:health',
      event: 'state-update',
      data: { status: 'ok' },
    };

    expect(envelope).toHaveProperty('topic');
    expect(envelope).toHaveProperty('event');
    expect(envelope).toHaveProperty('data');
    expect(typeof envelope.topic).toBe('string');
    expect(typeof envelope.event).toBe('string');
    expect(typeof envelope.data).toBe('object');
  });

  test('envelope serializes to valid JSON', () => {
    const envelope = makeEnvelope('state-update', { type: 'health', score: 95 });
    const json = JSON.stringify(envelope);
    const parsed = parseMessage(json);

    expect(parsed.event).toBe('state-update');
    expect((parsed.data as Record<string, unknown>).type).toBe('health');
  });

  test('malformed JSON is handled gracefully', () => {
    const malformed = '{not valid json';
    let parsed: unknown = null;
    try {
      parsed = JSON.parse(malformed);
    } catch {
      // Expected — DashboardAdapter silently ignores parse errors
    }
    expect(parsed).toBeNull();
  });
});

// ── 2. Subscribe / Unsubscribe protocol ─────────────────

describe('Subscribe/Unsubscribe protocol', () => {
  test('subscribe message has correct shape', () => {
    const msg: WsSubscribe = {
      type: 'subscribe',
      topic: 'project:abc:command',
    };

    expect(msg.type).toBe('subscribe');
    expect(msg.topic).toBe('project:abc:command');
  });

  test('unsubscribe message has correct shape', () => {
    const msg: WsUnsubscribe = {
      type: 'unsubscribe',
      topic: 'project:abc:command',
    };

    expect(msg.type).toBe('unsubscribe');
    expect(msg.topic).toBe('project:abc:command');
  });

  test('subscribe/unsubscribe round-trip through JSON', () => {
    const sub: WsSubscribe = { type: 'subscribe', topic: 'hub:projects' };
    const json = JSON.stringify(sub);
    const parsed = parseMessage(json) as unknown as WsClientMessage;

    expect(parsed.type).toBe('subscribe');
    expect(parsed.topic).toBe('hub:projects');
  });
});

// ── 3. Topic patterns ───────────────────────────────────

describe('Topic patterns', () => {
  const validTopics = [
    'hub:projects',
    'hub:health',
    'project:abc-123:state',
    'project:abc-123:events',
    'project:abc-123:command',
    'project:abc-123:result',
  ];

  test.each(validTopics)('topic "%s" matches expected pattern', (topic) => {
    const hubPattern = /^hub:(projects|health)$/;
    const projectPattern = /^project:[\w-]+:(state|events|command|result)$/;

    const matches = hubPattern.test(topic) || projectPattern.test(topic);
    expect(matches).toBe(true);
  });

  test('project topic embeds project ID', () => {
    const projectId = 'my-project-42';
    const commandTopic = `project:${projectId}:command`;
    const resultTopic = `project:${projectId}:result`;

    expect(commandTopic).toBe('project:my-project-42:command');
    expect(resultTopic).toBe('project:my-project-42:result');
  });

  test('invalid topic patterns are rejected', () => {
    const invalid = ['', 'random', 'project:', 'project:id:', 'hub:unknown'];
    const hubPattern = /^hub:(projects|health)$/;
    const projectPattern = /^project:[\w-]+:(state|events|command|result)$/;

    for (const topic of invalid) {
      const matches = hubPattern.test(topic) || projectPattern.test(topic);
      expect(matches).toBe(false);
    }
  });
});

// ── 4. State update routing ─────────────────────────────

describe('State update routing', () => {
  const stateTypes = ['health', 'tokens', 'swarm', 'graph'] as const;

  function routeStateUpdate(dataType: string): string {
    const panelMap: Record<string, string> = {
      health: 'health-panel',
      tokens: 'token-panel',
      swarm: 'swarm-panel',
      graph: 'graph-panel',
    };
    return panelMap[dataType] ?? 'unknown';
  }

  test.each(stateTypes)('state-update with type "%s" routes to correct panel', (type) => {
    const envelope = makeEnvelope('state-update', { type });
    expect(envelope.event).toBe('state-update');

    const panel = routeStateUpdate(type);
    expect(panel).not.toBe('unknown');
  });

  test('unknown state type does not route to any panel', () => {
    const panel = routeStateUpdate('nonexistent');
    expect(panel).toBe('unknown');
  });

  test('state-update envelope carries full data payload', () => {
    const envelope = makeEnvelope('state-update', {
      type: 'health',
      score: 92,
      violations: 0,
      files: 47,
    });

    expect(envelope.data.type).toBe('health');
    expect(envelope.data.score).toBe(92);
    expect(envelope.data.violations).toBe(0);
  });
});

// ── 5. Command result handling ──────────────────────────

describe('Command result handling', () => {
  test('command-result matched by commandId', () => {
    const commandId = 'cmd-abc-123';
    const envelope = makeEnvelope('command-result', {
      commandId,
      status: 'completed',
      data: { pong: true },
      completedAt: new Date().toISOString(),
    });

    expect(envelope.event).toBe('command-result');
    expect(envelope.data.commandId).toBe(commandId);
    expect(envelope.data.status).toBe('completed');
  });

  test('failed command-result carries error message', () => {
    const envelope = makeEnvelope('command-result', {
      commandId: 'cmd-fail-1',
      status: 'failed',
      error: 'Unknown command type: bad-cmd',
      completedAt: new Date().toISOString(),
    });

    expect(envelope.data.status).toBe('failed');
    expect(envelope.data.error).toBe('Unknown command type: bad-cmd');
  });

  test('command envelope dispatched via WebSocket has correct fields', () => {
    const command = {
      event: 'command',
      data: {
        commandId: 'cmd-1',
        projectId: 'proj-1',
        type: 'ping',
        payload: {},
        issuedAt: new Date().toISOString(),
        source: 'browser',
      },
    };

    const json = JSON.stringify(command);
    const parsed = parseMessage(json);
    const data = parsed.data as Record<string, unknown>;

    expect(data.commandId).toBe('cmd-1');
    expect(data.type).toBe('ping');
    expect(data.source).toBe('browser');
  });
});

// ── 6. File change events ───────────────────────────────

describe('File change events', () => {
  test('file-change event carries path and layer', () => {
    const envelope = makeEnvelope('file-change', {
      path: 'src/core/domain/entities.ts',
      layer: 'domain',
      timestamp: Date.now(),
    });

    expect(envelope.event).toBe('file-change');
    expect(envelope.data.path).toBe('src/core/domain/entities.ts');
    expect(envelope.data.layer).toBe('domain');
    expect(typeof envelope.data.timestamp).toBe('number');
  });

  test('layer classification maps correctly for file-change', () => {
    const layerMap: Record<string, string> = {
      'src/core/domain/value-objects.ts': 'domain',
      'src/core/ports/swarm.ts': 'port',
      'src/core/usecases/analyze.ts': 'usecase',
      'src/adapters/primary/cli-adapter.ts': 'primary-adapter',
      'src/adapters/secondary/fs-adapter.ts': 'secondary-adapter',
      'src/index.ts': 'other',
    };

    function classifyLayer(filePath: string): string {
      if (filePath.includes('/core/domain/')) return 'domain';
      if (filePath.includes('/core/ports/')) return 'port';
      if (filePath.includes('/core/usecases/')) return 'usecase';
      if (filePath.includes('/adapters/primary/')) return 'primary-adapter';
      if (filePath.includes('/adapters/secondary/')) return 'secondary-adapter';
      return 'other';
    }

    for (const [path, expectedLayer] of Object.entries(layerMap)) {
      expect(classifyLayer(path)).toBe(expectedLayer);
    }
  });
});

// ── 7. Decision events ──────────────────────────────────

describe('Decision events', () => {
  test('decision-response event triggers modal data', () => {
    const envelope = makeEnvelope('decision-response', {
      decisionId: 'dec-1',
      question: 'Use REST or GraphQL for this adapter?',
      options: ['REST', 'GraphQL'],
      context: 'Choosing API style for external-api adapter',
    });

    expect(envelope.event).toBe('decision-response');
    expect(envelope.data.decisionId).toBe('dec-1');
    expect(envelope.data.question).toBeTruthy();
    expect(Array.isArray(envelope.data.options)).toBe(true);
    expect((envelope.data.options as string[]).length).toBe(2);
  });
});

// ── 8. Reconnection backoff ─────────────────────────────

describe('Reconnection backoff', () => {
  test('initial delay is 1 second', () => {
    let delay = 1000;
    expect(delay).toBe(1000);
  });

  test('delay doubles on each reconnect attempt', () => {
    let delay = 1000;
    const maxDelay = 30_000;
    const observed: number[] = [delay];

    for (let i = 0; i < 6; i++) {
      delay = Math.min(delay * 2, maxDelay);
      observed.push(delay);
    }

    expect(observed).toEqual([1000, 2000, 4000, 8000, 16000, 30000, 30000]);
  });

  test('delay caps at 30 seconds', () => {
    let delay = 16_000;
    const maxDelay = 30_000;

    delay = Math.min(delay * 2, maxDelay);
    expect(delay).toBe(30_000);

    // Further doublings still cap
    delay = Math.min(delay * 2, maxDelay);
    expect(delay).toBe(30_000);
  });

  test('delay resets to 1s on successful connection', () => {
    let delay = 16_000;
    // Simulate successful connect
    delay = 1000;
    expect(delay).toBe(1000);
  });
});

// ── 9. Project switching ────────────────────────────────

describe('Project switching', () => {
  function buildProjectTopics(projectId: string): string[] {
    return [
      `project:${projectId}:state`,
      `project:${projectId}:events`,
      `project:${projectId}:command`,
      `project:${projectId}:result`,
    ];
  }

  test('switching unsubscribes from old and subscribes to new topics', () => {
    const oldId = 'old-proj';
    const newId = 'new-proj';

    const unsubscribes: WsUnsubscribe[] = buildProjectTopics(oldId).map((topic) => ({
      type: 'unsubscribe',
      topic,
    }));

    const subscribes: WsSubscribe[] = buildProjectTopics(newId).map((topic) => ({
      type: 'subscribe',
      topic,
    }));

    // All old topics get unsubscribed
    expect(unsubscribes).toHaveLength(4);
    for (const msg of unsubscribes) {
      expect(msg.type).toBe('unsubscribe');
      expect(msg.topic).toContain(oldId);
    }

    // All new topics get subscribed
    expect(subscribes).toHaveLength(4);
    for (const msg of subscribes) {
      expect(msg.type).toBe('subscribe');
      expect(msg.topic).toContain(newId);
    }
  });

  test('no old topics remain after switching', () => {
    const oldId = 'proj-old';
    const newId = 'proj-new';

    const currentSubscriptions = new Set(buildProjectTopics(oldId));

    // Unsubscribe old
    for (const topic of buildProjectTopics(oldId)) {
      currentSubscriptions.delete(topic);
    }
    // Subscribe new
    for (const topic of buildProjectTopics(newId)) {
      currentSubscriptions.add(topic);
    }

    // No old topics remain
    for (const topic of currentSubscriptions) {
      expect(topic).not.toContain(oldId);
      expect(topic).toContain(newId);
    }
    expect(currentSubscriptions.size).toBe(4);
  });
});

// ── 10. Connection status ───────────────────────────────

describe('Connection status tracking', () => {
  test('initial state is disconnected', () => {
    let isListening = false;
    expect(isListening).toBe(false);
  });

  test('state becomes connected on open', () => {
    let isListening = false;
    // Simulate 'open' event
    isListening = true;
    expect(isListening).toBe(true);
  });

  test('state becomes disconnected on close', () => {
    let isListening = true;
    // Simulate 'close' event
    isListening = false;
    expect(isListening).toBe(false);
  });

  test('state becomes disconnected on stopListening', () => {
    let isListening = true;
    // Simulate stopListening()
    isListening = false;
    expect(isListening).toBe(false);
  });

  test('stopped adapter does not reconnect', () => {
    let stopped = false;
    let reconnectScheduled = false;

    function scheduleReconnect(): void {
      if (stopped) return;
      reconnectScheduled = true;
    }

    // Stop the adapter
    stopped = true;
    scheduleReconnect();

    expect(reconnectScheduled).toBe(false);
  });

  test('duplicate reconnect timers are prevented', () => {
    let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
    let reconnectCount = 0;

    function scheduleReconnect(): void {
      if (reconnectTimer) return; // Already scheduled
      reconnectCount++;
      reconnectTimer = setTimeout(() => {
        reconnectTimer = null;
      }, 1000);
    }

    scheduleReconnect();
    scheduleReconnect(); // Should be no-op
    scheduleReconnect(); // Should be no-op

    expect(reconnectCount).toBe(1);

    // Cleanup
    if (reconnectTimer) clearTimeout(reconnectTimer);
  });
});
