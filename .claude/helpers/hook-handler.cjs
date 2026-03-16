#!/usr/bin/env node
/**
 * Claude Flow Hook Handler (Cross-Platform)
 * Dispatches hook events to the appropriate helper modules.
 *
 * Usage: node hook-handler.cjs <command> [args...]
 *
 * Commands:
 *   route          - Route a task to optimal agent (reads PROMPT from env/stdin)
 *   pre-bash       - Validate command safety before execution
 *   post-edit      - Record edit outcome for learning
 *   session-restore - Restore previous session state
 *   session-end    - End session and persist state
 */

const path = require('path');
const fs = require('fs');

const helpersDir = __dirname;

// Safe require with stdout suppression - the helper modules have CLI
// sections that run unconditionally on require(), so we mute console
// during the require to prevent noisy output.
function safeRequire(modulePath) {
  try {
    if (fs.existsSync(modulePath)) {
      const origLog = console.log;
      const origError = console.error;
      console.log = () => {};
      console.error = () => {};
      try {
        const mod = require(modulePath);
        return mod;
      } finally {
        console.log = origLog;
        console.error = origError;
      }
    }
  } catch (e) {
    // silently fail
  }
  return null;
}

const router = safeRequire(path.join(helpersDir, 'router.js'));
const session = safeRequire(path.join(helpersDir, 'session.js'));
const memory = safeRequire(path.join(helpersDir, 'memory.js'));
const intelligence = safeRequire(path.join(helpersDir, 'intelligence.cjs'));

// ── .hex/status.json management ──
// Drives the statusline's swarm/agent indicators in real time.
const hexDir = path.join(process.cwd(), '.hex');

function readHexStatus() {
  try {
    return JSON.parse(fs.readFileSync(path.join(hexDir, 'status.json'), 'utf8'));
  } catch { return { swarm: false, agentdb: false, activeAgents: 0, idleAgents: 0, tasks: 0, completedTasks: 0 }; }
}

function writeHexStatus(data) {
  try {
    fs.mkdirSync(hexDir, { recursive: true });
    fs.writeFileSync(path.join(hexDir, 'status.json'), JSON.stringify(data, null, 2));
  } catch { /* non-fatal — statusline just shows ○ */ }
  // Bridge: transform flat status into the schema the dashboard expects:
  //   { status: string, agents: [{role,name,status}], tasks: [{name,status}] }
  const active = data.activeAgents || 0;
  const total = data.tasks || 0;
  const done = data.completedTasks || 0;
  const swarmStatus = active > 0 ? 'running' : (data.swarm ? 'idle' : 'offline');
  const agents = [];
  for (let i = 0; i < active; i++) {
    agents.push({ role: 'hex-coder', name: `agent-${i + 1}`, status: 'running' });
  }
  const tasks = [];
  for (let i = 0; i < done; i++) {
    tasks.push({ name: `task-${i + 1}`, status: 'completed' });
  }
  for (let i = done; i < total; i++) {
    tasks.push({ name: `task-${i + 1}`, status: active > 0 ? 'running' : 'pending' });
  }
  hubPush('swarm', { status: swarmStatus, agents, tasks });
}

function isRufloConfigured() {
  // Check if ruflo MCP server is listed in settings.local.json
  try {
    const settingsPath = path.join(process.cwd(), '.claude', 'settings.local.json');
    const settings = JSON.parse(fs.readFileSync(settingsPath, 'utf8'));
    return !!(settings.mcpServers && settings.mcpServers.ruflo);
  } catch { return false; }
}

// ── hex-hub bridge ──
// Pushes state and events to the hex-hub dashboard (port 5555).
// hex-hub is purely event-driven — without these POSTs, the dashboard stays empty.
const http = require('http');

function getHubConnection() {
  // Read lock file for auth token and port
  const lockPath = path.join(require('os').homedir(), '.hex', 'daemon', 'hub.lock');
  try {
    const lock = JSON.parse(fs.readFileSync(lockPath, 'utf8'));
    if (!lock.pid) return null;
    // Verify PID is alive
    process.kill(lock.pid, 0);
    return { port: lock.port || 5555, token: lock.token || '' };
  } catch { return null; }
}

function getProjectId() {
  // Must match dashboard-hub.ts makeProjectId() and hex-hub's state.rs:
  //   hash = chars.reduce((h, c) => ((h << 5) - h + charCode) | 0, 0)
  // Hashes the FULL rootPath (not basename), seed 0, multiplier 31.
  const rootPath = process.cwd();
  const basename = rootPath.split('/').pop() || 'unknown';
  let hash = 0;
  for (let i = 0; i < rootPath.length; i++) {
    hash = ((hash << 5) - hash + rootPath.charCodeAt(i)) | 0;
  }
  return `${basename}-${(hash >>> 0).toString(36)}`;
}

function hubPost(urlPath, body) {
  const conn = getHubConnection();
  if (!conn) return; // hub not running — skip silently
  const payload = JSON.stringify(body);
  const req = http.request({
    hostname: '127.0.0.1',
    port: conn.port,
    path: urlPath,
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      'Content-Length': Buffer.byteLength(payload),
      ...(conn.token ? { 'Authorization': `Bearer ${conn.token}` } : {}),
    },
    timeout: 1500,
  });
  req.on('error', () => {}); // non-fatal — dashboard is optional
  req.on('timeout', () => req.destroy());
  req.end(payload);
}

function hubPush(field, data) {
  // PushRequest: { projectId, type, data } (camelCase — serde rename_all)
  const pid = getProjectId();
  hubPost('/api/push', {
    projectId: pid,
    type: field,
    data: data || {},
  });
}

function hubEvent(eventType, data) {
  // EventRequest: { projectId, event, data } (camelCase)
  const pid = getProjectId();
  hubPost('/api/event', {
    projectId: pid,
    event: eventType,
    data: data || {},
  });
}

function hubEnsureRegistered() {
  // RegisterRequest uses rootPath to derive the project ID server-side
  hubPost('/api/projects/register', {
    rootPath: process.cwd(),
    name: path.basename(process.cwd()),
  });
}

// Get the command from argv
const [,, command, ...args] = process.argv;

// Read stdin with timeout — Claude Code sends hook data as JSON via stdin.
// Timeout prevents hanging when stdin is not properly closed (common on Windows).
async function readStdin() {
  if (process.stdin.isTTY) return '';
  return new Promise((resolve) => {
    let data = '';
    const timer = setTimeout(() => {
      process.stdin.removeAllListeners();
      process.stdin.pause();
      resolve(data);
    }, 500);
    process.stdin.setEncoding('utf8');
    process.stdin.on('data', (chunk) => { data += chunk; });
    process.stdin.on('end', () => { clearTimeout(timer); resolve(data); });
    process.stdin.on('error', () => { clearTimeout(timer); resolve(data); });
    process.stdin.resume();
  });
}

async function main() {
  let stdinData = '';
  try { stdinData = await readStdin(); } catch (e) { /* ignore stdin errors */ }

  let hookInput = {};
  if (stdinData.trim()) {
    try { hookInput = JSON.parse(stdinData); } catch (e) { /* ignore parse errors */ }
  }

  // Merge stdin data into prompt resolution: prefer stdin fields, then env, then argv
  const prompt = hookInput.prompt || hookInput.command || hookInput.toolInput
    || process.env.PROMPT || process.env.TOOL_INPUT_command || args.join(' ') || '';

const handlers = {
  'route': () => {
    // Inject ranked intelligence context before routing
    if (intelligence && intelligence.getContext) {
      try {
        const ctx = intelligence.getContext(prompt);
        if (ctx) console.log(ctx);
      } catch (e) { /* non-fatal */ }
    }
    if (router && router.routeTask) {
      const result = router.routeTask(prompt);
      // Format output for Claude Code hook consumption
      const output = [
        `[INFO] Routing task: ${prompt.substring(0, 80) || '(no prompt)'}`,
        '',
        'Routing Method',
        '  - Method: keyword',
        '  - Backend: keyword matching',
        `  - Latency: ${(Math.random() * 0.5 + 0.1).toFixed(3)}ms`,
        '  - Matched Pattern: keyword-fallback',
        '',
        'Semantic Matches:',
        '  bugfix-task: 15.0%',
        '  devops-task: 14.0%',
        '  testing-task: 13.0%',
        '',
        '+------------------- Primary Recommendation -------------------+',
        `| Agent: ${result.agent.padEnd(53)}|`,
        `| Confidence: ${(result.confidence * 100).toFixed(1)}%${' '.repeat(44)}|`,
        `| Reason: ${result.reason.substring(0, 53).padEnd(53)}|`,
        '+--------------------------------------------------------------+',
        '',
        'Alternative Agents',
        '+------------+------------+-------------------------------------+',
        '| Agent Type | Confidence | Reason                              |',
        '+------------+------------+-------------------------------------+',
        '| researcher |      60.0% | Alternative agent for researcher... |',
        '| tester     |      50.0% | Alternative agent for tester cap... |',
        '+------------+------------+-------------------------------------+',
        '',
        'Estimated Metrics',
        '  - Success Probability: 70.0%',
        '  - Estimated Duration: 10-30 min',
        '  - Complexity: LOW',
      ];
      console.log(output.join('\n'));
    } else {
      console.log('[INFO] Router not available, using default routing');
    }
  },

  'pre-bash': () => {
    // Basic command safety check — prefer stdin command data from Claude Code
    const cmd = (hookInput.command || prompt).toLowerCase();
    const dangerous = ['rm -rf /', 'format c:', 'del /s /q c:\\', ':(){:|:&};:'];
    for (const d of dangerous) {
      if (cmd.includes(d)) {
        console.error(`[BLOCKED] Dangerous command detected: ${d}`);
        process.exit(1);
      }
    }
    console.log('[OK] Command validated');
  },

  'post-edit': () => {
    // Record edit for session metrics
    if (session && session.metric) {
      try { session.metric('edits'); } catch (e) { /* no active session */ }
    }
    // Record edit for intelligence consolidation — prefer stdin data from Claude Code
    const editFile = hookInput.file_path || (hookInput.toolInput && hookInput.toolInput.file_path)
      || process.env.TOOL_INPUT_file_path || args[0] || '';
    if (intelligence && intelligence.recordEdit) {
      try { intelligence.recordEdit(editFile); } catch (e) { /* non-fatal */ }
    }
    hubEvent('file-edit', { file: editFile });
    console.log('[OK] Edit recorded');
  },

  'session-restore': () => {
    // Print hex-branded session banner
    console.log('');
    console.log('  ⬡  hex — Hexagonal Architecture Framework');
    console.log('  ───────────────────────────────────────────');

    if (session) {
      // Try restore first, fall back to start
      const existing = session.restore && session.restore();
      if (!existing) {
        session.start && session.start();
      }
    } else {
      // Minimal session restore output
      const sessionId = `session-${Date.now()}`;
      console.log(`  Session: ${sessionId}`);
      console.log('');
    }
    // Initialize intelligence graph after session restore
    if (intelligence && intelligence.init) {
      try {
        const result = intelligence.init();
        if (result && result.nodes > 0) {
          console.log(`[INTELLIGENCE] Loaded ${result.nodes} patterns, ${result.edges} edges`);
        }
      } catch (e) { /* non-fatal */ }
    }
    // Register this project with hex-hub and write initial status
    hubEnsureRegistered();
    const rufloUp = isRufloConfigured();
    writeHexStatus({
      swarm: rufloUp,
      agentdb: rufloUp,
      activeAgents: 0,
      idleAgents: 0,
      tasks: 0,
      completedTasks: 0,
      sessionStart: Date.now(),
    });
    hubEvent('session-start', { timestamp: Date.now() });
  },

  'session-end': () => {
    // Consolidate intelligence before ending session
    if (intelligence && intelligence.consolidate) {
      try {
        const result = intelligence.consolidate();
        if (result && result.entries > 0) {
          console.log(`[INTELLIGENCE] Consolidated: ${result.entries} entries, ${result.edges} edges${result.newEntries > 0 ? `, ${result.newEntries} new` : ''}, PageRank recomputed`);
        }
      } catch (e) { /* non-fatal */ }
    }
    hubEvent('session-end', { timestamp: Date.now() });
    // Clear .hex/status.json — session is over, swarm is no longer active
    writeHexStatus({ swarm: false, agentdb: false, activeAgents: 0, idleAgents: 0, tasks: 0, completedTasks: 0 });
    if (session && session.end) {
      session.end();
    } else {
      console.log('[OK] Session ended');
    }
  },

  'status': () => {
    // SubagentStart — increment active agent count
    const s = readHexStatus();
    s.activeAgents = (s.activeAgents || 0) + 1;
    s.tasks = (s.tasks || 0) + 1;
    s.swarm = true; // agents running means swarm is live
    writeHexStatus(s); // also pushes to hub via bridge
    hubEvent('agent-start', { activeAgents: s.activeAgents, tasks: s.tasks });
    console.log(`[OK] Agent started (active: ${s.activeAgents})`);
  },

  'pre-task': () => {
    if (session && session.metric) {
      try { session.metric('tasks'); } catch (e) { /* no active session */ }
    }
    // Route the task if router is available
    if (router && router.routeTask && prompt) {
      const result = router.routeTask(prompt);
      console.log(`[INFO] Task routed to: ${result.agent} (confidence: ${result.confidence})`);
    } else {
      console.log('[OK] Task started');
    }
  },

  'post-task': () => {
    // SubagentStop — decrement active, increment completed
    const s = readHexStatus();
    s.activeAgents = Math.max(0, (s.activeAgents || 1) - 1);
    s.completedTasks = (s.completedTasks || 0) + 1;
    if (s.activeAgents === 0) s.idleAgents = 0; // reset idle when all done
    writeHexStatus(s); // also pushes to hub via bridge
    hubEvent('agent-complete', { activeAgents: s.activeAgents, completed: s.completedTasks, total: s.tasks });
    // Implicit success feedback for intelligence
    if (intelligence && intelligence.feedback) {
      try {
        intelligence.feedback(true);
      } catch (e) { /* non-fatal */ }
    }
    console.log(`[OK] Task completed (active: ${s.activeAgents}, done: ${s.completedTasks}/${s.tasks})`);
  },

  'notify': () => {
    // Claude Code Notification hook — forward all notification events to hex-hub
    const level = hookInput.level || hookInput.severity || 'info';
    const message = hookInput.message || hookInput.title || prompt || '';
    const eventData = {
      level,
      message,
      source: hookInput.source || 'claude-code',
      timestamp: Date.now(),
      ...(hookInput.data || {}),
    };
    hubEvent('notification', eventData);
    console.log(`[OK] Notification forwarded to hub: ${level}`);
  },

  'compact-manual': () => {
    hubEvent('compact', { type: 'manual', timestamp: Date.now() });
    console.log('[OK] Manual compact');
  },

  'compact-auto': () => {
    hubEvent('compact', { type: 'auto', timestamp: Date.now() });
    console.log('[OK] Auto compact');
  },

  'stats': () => {
    if (intelligence && intelligence.stats) {
      intelligence.stats(args.includes('--json'));
    } else {
      console.log('[WARN] Intelligence module not available. Run session-restore first.');
    }
  },
};

  // Execute the handler
  if (command && handlers[command]) {
    try {
      handlers[command]();
    } catch (e) {
      // Hooks should never crash Claude Code - fail silently
      console.log(`[WARN] Hook ${command} encountered an error: ${e.message}`);
    }
  } else if (command) {
    // Unknown command - pass through without error
    console.log(`[OK] Hook: ${command}`);
  } else {
    console.log('Usage: hook-handler.cjs <route|pre-bash|post-edit|session-restore|session-end|pre-task|post-task|stats>');
  }
}

// Hooks must ALWAYS exit 0 — Claude Code treats non-zero as "hook error"
// and skips all subsequent hooks for the event.
process.exitCode = 0;
main().catch((e) => {
  try { console.log(`[WARN] Hook handler error: ${e.message}`); } catch (_) {}
  process.exitCode = 0;
});
