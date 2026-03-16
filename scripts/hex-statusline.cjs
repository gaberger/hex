#!/usr/bin/env node
/**
 * hex status line — shows framework connection status
 * 
 * Displays: project ID, git branch, swarm status, AgentDB, dashboard URL, health score
 * Install: hex setup (copies to project and adds to .claude/settings.json)
 */
const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

// ANSI
const c = {
  r: '\x1b[0m', b: '\x1b[1m', d: '\x1b[2m',
  grn: '\x1b[32m', ylw: '\x1b[33m', red: '\x1b[31m',
  cyn: '\x1b[36m', mag: '\x1b[35m', blu: '\x1b[34m',
};

function safe(fn, fb) { try { return fn(); } catch { return fb; } }

// ── Git branch
const branch = safe(() => execSync('git branch --show-current 2>/dev/null', { encoding: 'utf8' }).trim(), '?');

// ── Project identity from .hex/project.json
let projectId = '';
let projectName = '';
const projFile = path.join(process.cwd(), '.hex', 'project.json');
const projData = safe(() => JSON.parse(fs.readFileSync(projFile, 'utf8')), null);
if (projData) {
  projectId = projData.id || '';
  projectName = projData.name || '';
}
if (!projectName) {
  projectName = path.basename(process.cwd());
}

// ── Swarm + AgentDB status via claude-flow runtime files
// claude-flow runs as an MCP server in the same Claude Code session.
// Check its runtime state files rather than shelling out to npx.
let swarmUp = false;
let agents = 0;
let tasks = 0;
let dbUp = false;

const cfHome = path.join(require('os').homedir(), '.claude-flow');
const cfMetrics = path.join(cfHome, 'metrics');

// If claude-flow has a metrics file, the MCP server is active
if (safe(() => fs.existsSync(cfMetrics), false)) {
  swarmUp = true; // claude-flow MCP server is running = swarm available
}

// Check if claude-flow MCP is registered in this session's .mcp.json or parent
const mcpJson = safe(() => JSON.parse(fs.readFileSync(path.join(process.cwd(), '.mcp.json'), 'utf8')), null);
const hexMcpUp = mcpJson && mcpJson.mcpServers && mcpJson.mcpServers.hex;

// AgentDB: check if the bridge state file exists
const dbState = path.join(cfHome, 'agentdb.json');
if (safe(() => fs.existsSync(dbState), false)) {
  dbUp = true;
}
// Fallback: if claude-flow metrics exist and are recent, assume agentdb available
if (!dbUp && swarmUp) {
  const metricsAge = safe(() => Date.now() - fs.statSync(cfMetrics).mtimeMs, Infinity);
  if (metricsAge < 300000) dbUp = true; // metrics updated in last 5min
}

// ── Dashboard — check registry for assigned port
let dashUrl = '';
const regFile = path.join(require('os').homedir(), '.hex', 'registry.json');
const regData = safe(() => JSON.parse(fs.readFileSync(regFile, 'utf8')), null);
if (regData) {
  const projects = Array.isArray(regData) ? regData : (regData.projects || []);
  const match = projects.find(p => p.rootPath === process.cwd() || p.name === projectName);
  if (match && match.port) {
    dashUrl = `localhost:${match.port}`;
  }
}

// ── Health score from last analysis cache
let score = '';
const scoreFile = path.join(process.cwd(), '.hex', 'last-score.txt');
score = safe(() => fs.readFileSync(scoreFile, 'utf8').trim(), '');

// ── Build status line
const dot = (ok) => ok ? `${c.grn}●${c.r}` : `${c.d}○${c.r}`;
const scoreCol = !score ? c.d : parseInt(score) >= 80 ? c.grn : parseInt(score) >= 50 ? c.ylw : c.red;

// Line 1: hex identity + connections
const line1Parts = [
  `${c.b}${c.mag}hex${c.r}`,
  `${c.d}⎇${c.r} ${branch}`,
  `${dot(swarmUp)} swarm${agents ? ` ${c.cyn}${agents}a${c.r}` : ''}${tasks ? ` ${tasks}t` : ''}`,
  `${dot(dbUp)} agentdb`,
  dashUrl ? `${c.grn}◉${c.r} ${c.blu}${dashUrl}${c.r}` : `${c.d}○${c.r} dash`,
  score ? `${scoreCol}${score}${c.r}/100` : '',
].filter(Boolean);

// Pad line 1 past column 25 to avoid Claude Code collision zone
const line1 = '                          ' + line1Parts.join(`${c.d}  │  ${c.r}`);

// Line 2: project identity  
const idStr = projectId ? `${c.d}id:${c.r}${projectId.slice(0, 8)}` : '';
const line2Parts = [
  `${c.d}project:${c.r} ${c.b}${projectName}${c.r}`,
  idStr,
].filter(Boolean);
const line2 = '                          ' + line2Parts.join(`${c.d}  │  ${c.r}`);

process.stdout.write(line1 + '\n' + line2);
