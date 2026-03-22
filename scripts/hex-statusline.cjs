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
const activeAgents   = statusData ? (statusData.activeAgents || 0) : 0;
const idleAgents     = statusData ? (statusData.idleAgents || 0) : 0;
const totalTasks     = statusData ? (statusData.tasks || 0) : 0;
const completedTasks = statusData ? (statusData.completedTasks || 0) : 0;

// Check if hex-nexus daemon is running (lock file, status.json, or TCP port probe)
const hubLockPath = path.join(require('os').homedir(), '.hex', 'daemon', 'hub.lock');
const hubLock = safe(() => JSON.parse(fs.readFileSync(hubLockPath, 'utf8')), null);
const hubPidAlive = !!(hubLock && hubLock.pid && safe(() => { process.kill(hubLock.pid, 0); return true; }, false));
// Probe hex-nexus API health endpoint (confirms it's actually hex-nexus, not another service)
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

// HexFlo live status — fetch from hex-nexus REST API if daemon is running
let hexfloSwarms = 0, hexfloTasks = 0, hexfloTasksDone = 0, hexfloAgents = 0;
if (hubRunning) {
  const http = require('http');
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

// Git
const mark = isDirty ? `${P.dirty}✱` : `${P.clean}✓`;
parts.push(`${P.branch}⎇ ${branch} ${mark}`);

// HexFlo swarm — ●hexflo 2⚡ [3/5]
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
