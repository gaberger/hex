#!/usr/bin/env node
/**
 * hex status line — high-contrast bar with standard Unicode (no Nerd Fonts)
 *
 * Uses a single medium-grey background with bright foreground colors
 * so it reads clearly on both dark and light terminals.
 *
 * Reads .hex/status.json and .hex/project.json for state.
 * Uses execFileSync for git (no shell, no injection risk).
 */
const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const ESC = '\x1b';
const R   = `${ESC}[0m`;
const B   = `${ESC}[1m`;
const D   = `${ESC}[2m`;
const fg  = (n) => `${ESC}[38;5;${n}m`;
const bg  = (n) => `${ESC}[48;5;${n}m`;

// Bar background — medium grey (visible on dark AND light terminals)
const BAR = bg(237);

// High-contrast foreground palette on grey background
const P = {
  brand:   `${B}${fg(177)}`,  // bright purple — hex identity
  project: `${B}${fg(81)}`,   // bright cyan
  branch:  `${B}${fg(255)}`,  // white
  dirty:   `${B}${fg(220)}`,  // bright yellow
  clean:   `${B}${fg(84)}`,   // bright green
  active:  `${B}${fg(84)}`,   // bright green
  idle:    `${fg(220)}`,       // yellow
  dim:     `${fg(243)}`,       // mid grey
  on:      `${B}${fg(84)}`,   // bright green dot
  off:     `${fg(250)}`,       // lighter grey dot (visible on dark bg)
  sep:     `${fg(245)}`,       // separator color
  scoreOk: `${B}${fg(84)}`,
  scoreWn: `${B}${fg(220)}`,
  scoreBd: `${B}${fg(203)}`,
  adr:     `${B}${fg(213)}`,       // bright magenta-pink — active ADR
};

function safe(fn, fb) { try { return fn(); } catch { return fb; } }

// ─── Data collection ─────────────────────────────────────────────
const cwd = process.cwd();
const hexDir = path.join(cwd, '.hex');

const branch = safe(() =>
  execFileSync('git', ['branch', '--show-current'],
    { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trim(), '?');

const isDirty = safe(() => {
  const s = execFileSync('git', ['status', '--porcelain'],
    { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] }).trim();
  return s.length > 0;
}, false);

const projData = safe(() =>
  JSON.parse(fs.readFileSync(path.join(hexDir, 'project.json'), 'utf8')), null);
const projectName = (projData && projData.name) || path.basename(cwd);

const statusData = safe(() =>
  JSON.parse(fs.readFileSync(path.join(hexDir, 'status.json'), 'utf8')), null);
const swarmUp        = statusData ? !!statusData.swarm : false;
const agentdbUp      = statusData ? !!statusData.agentdb : false;
const dashUrl        = statusData ? (statusData.dashboard || '') : '';
const dashProjectId  = statusData ? (statusData.projectId || '') : '';
const activeAdr      = statusData ? (statusData.activeAdr || '') : '';
const activeAdrTitle = statusData ? (statusData.activeAdrTitle || '') : '';
const activeAgents   = statusData ? (statusData.activeAgents || 0) : 0;
const idleAgents     = statusData ? (statusData.idleAgents || 0) : 0;
const totalTasks     = statusData ? (statusData.tasks || 0) : 0;
const completedTasks = statusData ? (statusData.completedTasks || 0) : 0;

// Check if hex nexus daemon is running (lock file, status.json, or TCP port probe)
const hubLockPath = path.join(require('os').homedir(), '.hex', 'daemon', 'hub.lock');
const hubLock = safe(() => JSON.parse(fs.readFileSync(hubLockPath, 'utf8')), null);
const hubPidAlive = !!(hubLock && hubLock.pid && safe(() => { process.kill(hubLock.pid, 0); return true; }, false));
// Probe hex nexus API health endpoint (confirms it's actually hex nexus, not another service)
const nexusAlive = safe(() => {
  const out = execFileSync('node', ['-e', `
    const http = require('http');
    const req = http.get('http://127.0.0.1:5555/api/health', {timeout: 1000}, (res) => {
      process.stdout.write(String(res.statusCode));
    });
    req.on('error', () => process.stdout.write('0'));
    req.on('timeout', () => { req.destroy(); process.stdout.write('0'); });
  `], { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'], timeout: 1500 });
  return parseInt(out, 10) >= 200 && parseInt(out, 10) < 500;
}, false);
const hubRunning = hubPidAlive || nexusAlive;

const cfMetrics = path.join(require('os').homedir(), '.claude-flow', 'metrics');
const cfAlive = safe(() =>
  fs.existsSync(cfMetrics) && (Date.now() - fs.statSync(cfMetrics).mtimeMs) < 300000, false);

const settingsLocal = safe(() =>
  JSON.parse(fs.readFileSync(path.join(cwd, '.claude', 'settings.local.json'), 'utf8')), null);
const hexMcpConfigured = !!(settingsLocal && settingsLocal.mcpServers && settingsLocal.mcpServers.hex);

// Agent identity — match THIS Claude instance by walking PPID chain
const sessDir = path.join(require('os').homedir(), '.hex', 'sessions');

// Collect ancestor PIDs up to init (walk PPID chain via `ps`)
const ancestorPids = safe(() => {
  const out = execFileSync('ps', ['-o', 'pid=,ppid=,comm=', '-ax'],
    { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'] });
  const procs = new Map();
  for (const line of out.trim().split('\n')) {
    const parts = line.trim().split(/\s+/);
    if (parts.length >= 3) procs.set(parts[0], { ppid: parts[1], comm: parts.slice(2).join(' ') });
  }
  // Walk from our PID up to find the `claude` process
  const pids = [];
  let cur = String(process.pid);
  for (let i = 0; i < 10 && cur && cur !== '0' && cur !== '1'; i++) {
    const p = procs.get(cur);
    if (!p) break;
    pids.push(cur);
    cur = p.ppid;
  }
  return pids;
}, []);

const claudeSessionId = process.env.CLAUDE_SESSION_ID || '';

const agentSession = safe(() => {
  // Strategy 0: CLAUDE_SESSION_ID — the canonical session identifier
  if (claudeSessionId) {
    const sessionFile = path.join(sessDir, `agent-${claudeSessionId}.json`);
    if (fs.existsSync(sessionFile)) {
      return JSON.parse(fs.readFileSync(sessionFile, 'utf8'));
    }
  }

  const files = fs.readdirSync(sessDir)
    .filter(f => f.startsWith('agent-') && !f.includes('nexus') && f.endsWith('.json'))
    .map(f => {
      const data = JSON.parse(fs.readFileSync(path.join(sessDir, f), 'utf8'));
      return { name: f, data, mtime: fs.statSync(path.join(sessDir, f)).mtimeMs };
    });
  if (files.length === 0) return null;

  // Strategy 1: match by claude_pid in ancestor chain (unique per instance)
  if (ancestorPids.length > 0) {
    const match = files.find(f =>
      f.data.claude_pid && ancestorPids.includes(String(f.data.claude_pid))
    );
    if (match) return match.data;
  }

  // Strategy 2: fallback to newest (legacy session files without claude_pid)
  files.sort((a, b) => b.mtime - a.mtime);
  return files[0].data;
}, null);
const agentId = agentSession ? agentSession.agentId : null;
const agentName = agentSession ? agentSession.name : null;
const agentIdShort = agentId ? agentId.slice(0, 8) : null;

// HexFlo live status — fetch from hex nexus REST API if daemon is running
let hexfloSwarms = 0, hexfloTasks = 0, hexfloTasksDone = 0, hexfloAgents = 0;
let pulseProjects = [];
// Brain autonomous supervisor — queue depth + last test timestamp
let brainQueue = 0, brainRunning = 0, brainLastTest = 'never';
if (hubRunning) {
  const fetchSync = (urlPath) => safe(() => {
    const result = execFileSync('node', ['-e', `
      const http = require('http');
      const req = http.get('http://127.0.0.1:5555${urlPath}', {timeout: 1500}, (res) => {
        let d = '';
        res.on('data', c => d += c);
        res.on('end', () => process.stdout.write(d));
      });
      req.on('error', () => process.stdout.write('{}'));
      req.on('timeout', () => { req.destroy(); process.stdout.write('{}'); });
    `], { encoding: 'utf8', stdio: ['pipe', 'pipe', 'pipe'], timeout: 2000 });
    return JSON.parse(result || '{}');
  }, {});

  const swarmsData = fetchSync('/api/swarms/active');
  if (Array.isArray(swarmsData)) {
    hexfloSwarms = swarmsData.length;
    for (const s of swarmsData) {
      const tasks = Array.isArray(s.tasks) ? s.tasks : [];
      hexfloTasks += tasks.length;
      hexfloTasksDone += tasks.filter(t => t.status === 'completed').length;
    }
  }

  // Pulse — per-project state from /api/pulse (ADR-2604131500 P6.2)
  const pulseData = fetchSync('/api/pulse');
  if (Array.isArray(pulseData)) {
    pulseProjects = pulseData;
  }

  // Brain — queue depth + running + last test age so operators see autonomous work.
  // Scope by project so one repo's queue doesn't pollute another's statusline.
  const hexProjectJson = safe(() =>
    JSON.parse(fs.readFileSync(path.join(cwd, '.hex', 'project.json'), 'utf8')), null);
  const hexProjectId = hexProjectJson && hexProjectJson.id;
  const brainUrl = hexProjectId
    ? `/api/brain/status?project=${encodeURIComponent(hexProjectId)}`
    : '/api/brain/status';
  const brainData = fetchSync(brainUrl);
  if (brainData && typeof brainData === 'object') {
    brainQueue = typeof brainData.queue_pending === 'number' ? brainData.queue_pending : 0;
    brainRunning = typeof brainData.queue_running === 'number' ? brainData.queue_running : 0;
    brainLastTest = brainData.last_test || 'never';
  }
}

const swarmShow = swarmUp || cfAlive || hexMcpConfigured || hexfloSwarms > 0;
const dbShow = agentdbUp || cfAlive;

const score = safe(() =>
  fs.readFileSync(path.join(hexDir, 'last-score.txt'), 'utf8').trim(), '');

const pkgJson = safe(() =>
  JSON.parse(fs.readFileSync(path.join(cwd, 'package.json'), 'utf8')), null);
const hexVersion = (pkgJson && pkgJson.version) || '';

const mcpJson = safe(() =>
  JSON.parse(fs.readFileSync(path.join(cwd, '.mcp.json'), 'utf8')), null);
const hexMcp = !!(mcpJson && mcpJson.mcpServers && mcpJson.mcpServers.hex);

// ─── Build segments ──────────────────────────────────────────────
const sep = `${BAR}${P.sep} │ `;
const parts = [];

// Brand (no version — matches README format)
parts.push(`${P.brand}⬡ hex`);

// Project
parts.push(`${P.project}${projectName}`);

// Agent identity — always show short ID for uniqueness across multiple agents
if (agentIdShort) {
  const label = agentName ? `${agentName}:${agentIdShort}` : agentIdShort;
  parts.push(`${P.dim}⚙ ${label}`);
}

// Git
const mark = isDirty ? `${P.dirty}✱` : `${P.clean}✓`;
parts.push(`${P.branch}⎇ ${branch} ${mark}`);

// Active ADR — ◆ADR-2603240130 Declarative Swarm…
if (activeAdr) {
  const title = activeAdrTitle.length > 28
    ? activeAdrTitle.slice(0, 27) + '…'
    : activeAdrTitle;
  const label = title ? `${activeAdr} ${P.dim}${title}` : activeAdr;
  parts.push(`${P.adr}◆${label}`);
}

// Pulse — per-project state (ADR-2604131500 P6.2)
// Symbols: ● active (green), ◐ decision (yellow), ◉ blocked (red), ○ idle (dim), ✓ complete (green)
const pulseSymbol = (state) => {
  switch (state) {
    case 'active':   return `${P.active}●`;
    case 'decision': return `${P.idle}◐`;
    case 'blocked':  return `${P.scoreBd}◉`;
    case 'complete': return `${P.active}✓`;
    default:         return `${P.dim}○`;
  }
};

if (pulseProjects.length > 0) {
  const maxShow = 4;
  const shown = pulseProjects.slice(0, maxShow);
  const pulseParts = shown.map(p => {
    const name = (p.name || p.project_id || '?').length > 12
      ? (p.name || p.project_id || '?').slice(0, 11) + '…'
      : (p.name || p.project_id || '?');
    const agents = p.agent_count > 0 ? `${p.agent_count}⚡` : '';
    const decs = p.decision_count > 0 ? `${P.idle}${p.decision_count}?` : '';
    return `${pulseSymbol(p.state)}${name}${agents ? ' ' + agents : ''}${decs ? ' ' + decs : ''}`;
  });
  const extra = pulseProjects.length > maxShow ? ` ${P.dim}+${pulseProjects.length - maxShow}` : '';
  parts.push(pulseParts.join(' ') + extra);
} else {
  // Fallback to legacy HexFlo swarm display
  const agt = activeAgents || hexfloAgents;
  const tTotal = totalTasks || hexfloTasks;
  const tDone = completedTasks || hexfloTasksDone;
  const nSwarms = hexfloSwarms;

  if (agt > 0 || nSwarms > 0) {
    const agentCounts = agt > 0 ? ` ${agt}⚡` : '';
    const swarmCount = nSwarms > 1 ? ` ${nSwarms}▣` : '';
    const tasks = tTotal ? ` [${tDone}/${tTotal}]` : '';
    parts.push(`${P.active}●hexflo${swarmCount}${agentCounts}${P.branch}${tasks}`);
  } else if (swarmShow) {
    const idleTag = idleAgents > 0 ? ` ${idleAgents}💤` : '';
    parts.push(`${P.idle}●hexflo${idleTag}`);
  } else {
    parts.push(`${P.dim}○hexflo`);
  }
}

// Brain autonomous supervisor — ◉brain or ○brain, with queue depth if > 0.
// Daemon liveness is detected via the PID file + process check (kill -0).
const brainPidFile = path.join(require('os').homedir(), '.hex', 'brain-daemon.pid');
let brainDaemonAlive = false;
try {
  const rawPid = fs.readFileSync(brainPidFile, 'utf8').trim();
  const pid = parseInt(rawPid, 10);
  if (pid > 0) {
    try { process.kill(pid, 0); brainDaemonAlive = true; } catch { /* stale pid */ }
  }
} catch { /* no pid file — daemon not running */ }

const brainDot = brainDaemonAlive ? `${P.on}◉` : `${P.off}○`;
const brainRunTag = brainRunning > 0 ? `${P.active} ${brainRunning}▶` : '';
const brainQueueTag = brainQueue > 0 ? `${P.branch} ${brainQueue}⤵` : '';
parts.push(`${brainDot}brain${brainRunTag}${brainQueueTag}`);

// Services — README format: ●db │ ◉localhost:3456 │ ◉mcp
const svcDot = (on, label) => on ? `${P.on}◉${label}` : `${P.off}○${label}`;
parts.push(svcDot(dbShow, 'db'));

// Nexus daemon — show clickable host:port when running, clear "offline" when not
const nexusActive = hubRunning || !!dashUrl;
const nexusPort = (hubLock && hubLock.port) || 5555;
const nexusHash = dashProjectId ? `#/project/${dashProjectId}` : '';
const nexusLink = `http://localhost:${nexusPort}/${nexusHash}`;
if (nexusActive) {
  parts.push(`${P.on}◉nexus ${ESC}]8;;${nexusLink}${ESC}\\:${nexusPort}${ESC}]8;;${ESC}\\`);
} else {
  parts.push(`${P.off}○nexus${P.dim} offline`);
}

parts.push(svcDot(hexMcp, 'mcp'));

// Health score — README format: 87/100
if (score) {
  const s = parseInt(score);
  const col = s >= 80 ? P.scoreOk : s >= 50 ? P.scoreWn : P.scoreBd;
  parts.push(`${col}${score}/100`);
}

// ─── Render ──────────────────────────────────────────────────────
const line = `${BAR} ${parts.join(sep)} ${R}`;
process.stdout.write('  ' + line);
