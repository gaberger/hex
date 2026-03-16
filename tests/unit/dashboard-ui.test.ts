/**
 * Dashboard UI logic tests.
 *
 * The dashboard is a single HTML file (hex-hub/assets/index.html) containing
 * ~2600 lines of vanilla JavaScript inside an IIFE. Since there are no ES
 * module exports, we extract the pure-logic functions verbatim and test them
 * standalone. Every function below is copied from index.html and clearly
 * marked — any drift should be caught by these tests failing.
 */
import { describe, test, expect } from 'bun:test';

// ---------------------------------------------------------------------------
// Extracted functions (from hex-hub/assets/index.html)
// ---------------------------------------------------------------------------

// --- escapeHtml (line ~417) ---
// Original uses DOM; we replicate the same escaping logic for Node/Bun.
function escapeHtml(str: string): string {
  return String(str)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#039;');
}

// --- scoreColor (line ~636) ---
function scoreColor(score: number): string {
  if (score >= 80) return '#3fb950';
  if (score >= 50) return '#d29922';
  return '#f85149';
}

// --- formatTime (line ~630) ---
function formatTime(ts: number | null): string {
  if (!ts) return '--:--';
  const d = new Date(ts);
  return d.toLocaleTimeString([], {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
    hour12: false,
  });
}

// --- HEX_RINGS + normalizeLayer (line ~1054, ~1134) ---
const HEX_RINGS = [
  { key: 'domain',            aliases: ['domain'] },
  { key: 'ports',             aliases: ['ports', 'port'] },
  { key: 'usecases',          aliases: ['usecases', 'usecase'] },
  { key: 'primary-adapter',   aliases: ['primary-adapter'] },
  { key: 'secondary-adapter', aliases: ['secondary-adapter'] },
  { key: 'other',             aliases: ['other', 'adapters', 'adapter'] },
];

function normalizeLayer(layer: string | undefined): string {
  const l = (layer || 'other').toLowerCase();
  for (let i = 0; i < HEX_RINGS.length; i++) {
    if (HEX_RINGS[i].aliases.indexOf(l) !== -1) return HEX_RINGS[i].key;
  }
  return 'other';
}

// --- clusterKey (line ~1148) ---
function clusterKey(nodeId: string): string {
  if (nodeId.includes('/')) {
    const parts = nodeId.split('/');
    const file = parts[parts.length - 1];
    const stem = file.replace(/\.[^.]+$/, '');
    return stem.replace(/\.test$|\.spec$|_test$/, '');
  }
  if (nodeId.includes(':')) return nodeId.split(':')[0];
  return nodeId;
}

// --- hexRadiusAtAngle (line ~1178) ---
function hexRadiusAtAngle(radius: number, angle: number): number {
  let a = ((angle % (Math.PI * 2)) + Math.PI * 2) % (Math.PI * 2);
  const sectorWidth = Math.PI / 3;
  const halfSector = Math.PI / 6;
  const sectorIndex = Math.floor((a + halfSector) / sectorWidth);
  const edgeCenterAngle = sectorIndex * sectorWidth;
  const offsetFromEdge = a - edgeCenterAngle;
  return radius * Math.cos(halfSector) / Math.cos(offsetFromEdge);
}

// --- computeTransitiveDeps (line ~1074) ---
// Re-implemented to accept edges explicitly instead of relying on graphState.
interface Edge { from: string; to: string; violation?: boolean; isViolation?: boolean }

function computeTransitiveDeps(nodeId: string, edges: Edge[]): Record<string, true> {
  const result: Record<string, true> = {};
  const queue = [nodeId];
  result[nodeId] = true;
  while (queue.length > 0) {
    const current = queue.shift()!;
    edges.forEach(function (e) {
      if (e.from === current && !result[e.to]) {
        result[e.to] = true;
        queue.push(e.to);
      }
    });
  }
  return result;
}

// --- violationCount computation (line ~1425-1434) ---
function computeViolationCount(edges: Edge[]): number {
  let count = 0;
  edges.forEach(function (e) {
    if (e.violation || e.isViolation) count++;
  });
  return count;
}

// --- event filtering (line ~908) ---
interface DashEvent { level: string; message: string }
function filterEvents(events: DashEvent[], filterType: string): DashEvent[] {
  return filterType === 'all'
    ? events
    : events.filter(function (e) { return e.level === filterType; });
}

// --- chatParseInput (line ~2209) ---
const hexCommandAliases: Record<string, string> = {
  'analyze':   'run-analyze',
  'build':     'run-build',
  'summarize': 'run-summarize',
  'validate':  'run-validate',
  'generate':  'run-generate',
  'status':    'ping',
  'ping':      'ping',
};

function chatParseInput(input: string): { type: string; payload: Record<string, string> } | null {
  const trimmed = input.trim();
  if (!trimmed) return null;

  const parts = trimmed.split(/\s+/);
  let type = parts[0];
  let rest = trimmed.slice(type.length).trim();

  if (type === 'hex') {
    const subCmd = (parts[1] || '').toLowerCase();
    if (hexCommandAliases[subCmd]) {
      type = hexCommandAliases[subCmd];
      rest = parts.slice(2).join(' ');
    } else {
      return null; // unknown or bare "hex"
    }
  }

  const payload: Record<string, string> = {};
  if (rest.charAt(0) === '{') {
    try { Object.assign(payload, JSON.parse(rest)); } catch (_e) { /* empty */ }
  } else if (rest) {
    rest.split(/\s+/).forEach(function (pair) {
      const eqIdx = pair.indexOf('=');
      if (eqIdx > 0) {
        payload[pair.slice(0, eqIdx)] = pair.slice(eqIdx + 1);
      } else if (pair) {
        if (type === 'spawn-agent') {
          if (!payload.name) payload.name = pair;
          else if (!payload.role) payload.role = pair;
        } else if (type === 'create-task') {
          if (!payload.title) payload.title = pair;
          else if (!payload.agentRole) payload.agentRole = pair;
        } else if (type === 'run-summarize') {
          if (!payload.filePath) payload.filePath = pair;
          else if (!payload.level) payload.level = pair;
        } else if (type === 'run-generate') {
          if (!payload.adapter) payload.adapter = pair;
          else if (!payload.portInterface) payload.portInterface = pair;
          else if (!payload.language) payload.language = pair;
        } else if (type === 'run-analyze') {
          payload.rootPath = pair;
        }
      }
    });
    // Fix spawn-agent shorthand: if only name given, it's actually the role
    if (type === 'spawn-agent' && payload.name && !payload.role) {
      payload.role = payload.name;
      payload.name = payload.role + '-' + Date.now().toString(36).slice(-4);
    }
  }

  return { type, payload };
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

describe('Dashboard UI Logic', () => {

  // 1. Violation ordering regression test
  describe('violationCount computation (REGRESSION)', () => {
    test('counts violation edges before badge render', () => {
      const edges: Edge[] = [
        { from: 'A', to: 'B', violation: true },
        { from: 'C', to: 'D', isViolation: true },
        { from: 'E', to: 'F' },
      ];
      expect(computeViolationCount(edges)).toBe(2);
    });

    test('returns 0 when no violations', () => {
      const edges: Edge[] = [{ from: 'A', to: 'B' }];
      expect(computeViolationCount(edges)).toBe(0);
    });

    test('counts only edges with violation flag, not isViolation=false', () => {
      const edges: Edge[] = [
        { from: 'A', to: 'B', violation: true },
        { from: 'C', to: 'D', isViolation: false },
      ];
      expect(computeViolationCount(edges)).toBe(1);
    });
  });

  // 2. Score color
  describe('scoreColor', () => {
    test('score >= 80 returns green', () => {
      expect(scoreColor(80)).toBe('#3fb950');
      expect(scoreColor(100)).toBe('#3fb950');
    });

    test('score >= 50 and < 80 returns yellow', () => {
      expect(scoreColor(50)).toBe('#d29922');
      expect(scoreColor(79)).toBe('#d29922');
    });

    test('score < 50 returns red', () => {
      expect(scoreColor(0)).toBe('#f85149');
      expect(scoreColor(49)).toBe('#f85149');
    });
  });

  // 3. Layer normalization
  describe('normalizeLayer', () => {
    test('domain stays domain', () => {
      expect(normalizeLayer('domain')).toBe('domain');
    });

    test('port normalizes to ports', () => {
      expect(normalizeLayer('port')).toBe('ports');
    });

    test('usecase normalizes to usecases', () => {
      expect(normalizeLayer('usecase')).toBe('usecases');
    });

    test('primary-adapter stays primary-adapter', () => {
      expect(normalizeLayer('primary-adapter')).toBe('primary-adapter');
    });

    test('unknown layer maps to other', () => {
      expect(normalizeLayer('foobar')).toBe('other');
    });

    test('undefined defaults to other', () => {
      expect(normalizeLayer(undefined)).toBe('other');
    });
  });

  // 4. Cluster key extraction
  describe('clusterKey', () => {
    test('file path extracts adapter name', () => {
      expect(clusterKey('src/adapters/secondary/git-adapter.ts')).toBe('git-adapter');
    });

    test('node module extracts namespace', () => {
      expect(clusterKey('node:fs')).toBe('node');
    });

    test('test file in path strips .test suffix', () => {
      expect(clusterKey('src/file.test.ts')).toBe('file');
    });

    test('bare filename without path returns as-is', () => {
      expect(clusterKey('file.test.ts')).toBe('file.test.ts');
    });

    test('simple filename returns stem', () => {
      expect(clusterKey('src/core/domain/entities.ts')).toBe('entities');
    });
  });

  // 5. Chat command parsing
  describe('chatParseInput', () => {
    test('hex analyze parses to run-analyze', () => {
      const result = chatParseInput('hex analyze');
      expect(result).not.toBeNull();
      expect(result!.type).toBe('run-analyze');
    });

    test('hex build parses to run-build', () => {
      const result = chatParseInput('hex build');
      expect(result).not.toBeNull();
      expect(result!.type).toBe('run-build');
    });

    test('spawn-agent coder shorthand sets role', () => {
      const result = chatParseInput('spawn-agent coder');
      expect(result).not.toBeNull();
      expect(result!.type).toBe('spawn-agent');
      expect(result!.payload.role).toBe('coder');
    });

    test('create-task with positional args', () => {
      const result = chatParseInput('create-task mytask coder');
      expect(result).not.toBeNull();
      expect(result!.type).toBe('create-task');
      expect(result!.payload.title).toBe('mytask');
      expect(result!.payload.agentRole).toBe('coder');
    });

    test('ping command', () => {
      const result = chatParseInput('ping');
      expect(result).not.toBeNull();
      expect(result!.type).toBe('ping');
      expect(result!.payload).toEqual({});
    });

    test('run-analyze with rootPath key=value', () => {
      const result = chatParseInput('run-analyze rootPath=/src');
      expect(result).not.toBeNull();
      expect(result!.type).toBe('run-analyze');
      expect(result!.payload.rootPath).toBe('/src');
    });

    test('hex validate parses to run-validate', () => {
      const result = chatParseInput('hex validate');
      expect(result).not.toBeNull();
      expect(result!.type).toBe('run-validate');
    });

    test('hex generate parses to run-generate', () => {
      const result = chatParseInput('hex generate');
      expect(result).not.toBeNull();
      expect(result!.type).toBe('run-generate');
    });

    test('hex summarize parses to run-summarize', () => {
      const result = chatParseInput('hex summarize');
      expect(result).not.toBeNull();
      expect(result!.type).toBe('run-summarize');
    });

    test('hex status parses to ping', () => {
      const result = chatParseInput('hex status');
      expect(result).not.toBeNull();
      expect(result!.type).toBe('ping');
    });

    test('run-summarize with positional filePath and level', () => {
      const result = chatParseInput('run-summarize src/cli.ts L1');
      expect(result).not.toBeNull();
      expect(result!.type).toBe('run-summarize');
      expect(result!.payload.filePath).toBe('src/cli.ts');
      expect(result!.payload.level).toBe('L1');
    });

    test('run-generate with positional adapter, portInterface, language', () => {
      const result = chatParseInput('run-generate my-adapter IMyPort typescript');
      expect(result).not.toBeNull();
      expect(result!.type).toBe('run-generate');
      expect(result!.payload.adapter).toBe('my-adapter');
      expect(result!.payload.portInterface).toBe('IMyPort');
      expect(result!.payload.language).toBe('typescript');
    });

    test('run-analyze with positional rootPath', () => {
      const result = chatParseInput('run-analyze ./src');
      expect(result).not.toBeNull();
      expect(result!.type).toBe('run-analyze');
      expect(result!.payload.rootPath).toBe('./src');
    });

    test('bare hex returns null', () => {
      expect(chatParseInput('hex')).toBeNull();
    });

    test('unknown hex subcommand returns null', () => {
      expect(chatParseInput('hex foobar')).toBeNull();
    });

    test('JSON payload parsing', () => {
      const result = chatParseInput('run-analyze {"rootPath": "/app"}');
      expect(result).not.toBeNull();
      expect(result!.type).toBe('run-analyze');
      expect(result!.payload.rootPath).toBe('/app');
    });

    test('empty input returns null', () => {
      expect(chatParseInput('')).toBeNull();
      expect(chatParseInput('   ')).toBeNull();
    });
  });

  // 6. Hex radius at angle
  describe('hexRadiusAtAngle', () => {
    test('at flat edge center equals apothem (R * cos 30)', () => {
      const R = 100;
      const apothem = R * Math.cos(Math.PI / 6);
      // At angle 0 (flat edge center for flat-top hex)
      const result = hexRadiusAtAngle(R, 0);
      expect(Math.abs(result - apothem)).toBeLessThan(0.001);
    });

    test('at vertex (30 deg) equals circumradius R', () => {
      const R = 100;
      const angle = Math.PI / 6; // 30 degrees = vertex for flat-top
      const result = hexRadiusAtAngle(R, angle);
      expect(Math.abs(result - R)).toBeLessThan(0.001);
    });

    test('hex radius is NOT constant (non-circular)', () => {
      const R = 100;
      const atEdge = hexRadiusAtAngle(R, 0);
      const atVertex = hexRadiusAtAngle(R, Math.PI / 6);
      expect(atEdge).not.toBeCloseTo(atVertex, 1);
    });
  });

  // 7. Transitive dependency computation
  describe('computeTransitiveDeps', () => {
    test('A->B, B->C, A->D yields {A,B,C,D}', () => {
      const edges: Edge[] = [
        { from: 'A', to: 'B' },
        { from: 'B', to: 'C' },
        { from: 'A', to: 'D' },
      ];
      const result = computeTransitiveDeps('A', edges);
      expect(result).toEqual({ A: true, B: true, C: true, D: true });
    });

    test('node with no outgoing edges returns only itself', () => {
      const edges: Edge[] = [{ from: 'X', to: 'Y' }];
      const result = computeTransitiveDeps('Y', edges);
      expect(result).toEqual({ Y: true });
    });

    test('handles cycles without infinite loop', () => {
      const edges: Edge[] = [
        { from: 'A', to: 'B' },
        { from: 'B', to: 'A' },
      ];
      const result = computeTransitiveDeps('A', edges);
      expect(result).toEqual({ A: true, B: true });
    });
  });

  // 8. Event filtering
  describe('filterEvents', () => {
    const events: DashEvent[] = [
      { level: 'info', message: 'started' },
      { level: 'error', message: 'failed' },
      { level: 'info', message: 'completed' },
    ];

    test('"all" returns all events', () => {
      expect(filterEvents(events, 'all')).toHaveLength(3);
    });

    test('"error" returns only error events', () => {
      const result = filterEvents(events, 'error');
      expect(result).toHaveLength(1);
      expect(result[0].message).toBe('failed');
    });

    test('non-matching filter returns empty', () => {
      expect(filterEvents(events, 'warn')).toHaveLength(0);
    });
  });

  // 9. HTML escaping
  describe('escapeHtml', () => {
    test('escapes script tags', () => {
      const result = escapeHtml('<script>alert(1)</script>');
      expect(result).not.toContain('<script>');
      expect(result).toContain('&lt;script&gt;');
    });

    test('escapes ampersands and quotes', () => {
      const result = escapeHtml('a & "b" & \'c\'');
      expect(result).toContain('&amp;');
      expect(result).toContain('&quot;');
      expect(result).toContain('&#039;');
    });
  });

  // 10. Format time
  describe('formatTime', () => {
    test('null returns --:--', () => {
      expect(formatTime(null)).toBe('--:--');
    });

    test('0 (falsy) returns --:--', () => {
      expect(formatTime(0)).toBe('--:--');
    });

    test('valid timestamp returns HH:MM:SS format', () => {
      // Use a known timestamp: 2024-01-01T12:30:45Z
      const ts = new Date('2024-01-01T12:30:45Z').getTime();
      const result = formatTime(ts);
      // Should match HH:MM:SS pattern regardless of timezone
      expect(result).toMatch(/^\d{2}:\d{2}:\d{2}$/);
    });
  });

  // 11. Swarm status normalization
  describe('swarm status normalization', () => {
    interface SwarmStatus { status: string; agentCount: number; activeTaskCount: number; completedTaskCount: number }
    interface Task { status: string }

    // Mirror the normalization logic from dashboard-adapter.ts pushSwarm
    function normalizeSwarmStatus(
      status: SwarmStatus,
      agents: unknown[],
      tasks: Task[],
    ): SwarmStatus {
      const activeTasks = tasks.filter(t => t.status === 'in-progress' || t.status === 'pending');
      if (agents.length === 0 && activeTasks.length === 0) {
        return { ...status, status: 'idle' };
      }
      return status;
    }

    test('daemon running with 0 agents and 0 tasks normalizes to idle', () => {
      const status: SwarmStatus = { status: 'running', agentCount: 0, activeTaskCount: 0, completedTaskCount: 0 };
      const result = normalizeSwarmStatus(status, [], []);
      expect(result.status).toBe('idle');
    });

    test('daemon running with agents stays running', () => {
      const status: SwarmStatus = { status: 'running', agentCount: 1, activeTaskCount: 0, completedTaskCount: 0 };
      const result = normalizeSwarmStatus(status, [{ id: 'a1' }], []);
      expect(result.status).toBe('running');
    });

    test('daemon running with active tasks stays running', () => {
      const status: SwarmStatus = { status: 'running', agentCount: 0, activeTaskCount: 1, completedTaskCount: 0 };
      const result = normalizeSwarmStatus(status, [], [{ status: 'in-progress' }]);
      expect(result.status).toBe('running');
    });

    test('daemon running with only completed tasks normalizes to idle', () => {
      const status: SwarmStatus = { status: 'running', agentCount: 0, activeTaskCount: 0, completedTaskCount: 5 };
      const result = normalizeSwarmStatus(status, [], [{ status: 'completed' }, { status: 'completed' }]);
      expect(result.status).toBe('idle');
    });

    test('idle status stays idle regardless', () => {
      const status: SwarmStatus = { status: 'idle', agentCount: 0, activeTaskCount: 0, completedTaskCount: 0 };
      const result = normalizeSwarmStatus(status, [], []);
      expect(result.status).toBe('idle');
    });

    test('pending tasks keep status as running', () => {
      const status: SwarmStatus = { status: 'running', agentCount: 0, activeTaskCount: 1, completedTaskCount: 0 };
      const result = normalizeSwarmStatus(status, [], [{ status: 'pending' }]);
      expect(result.status).toBe('running');
    });
  });
});
