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

// ── Swarm status via claude-flow
let swarmUp = false;
let agents = 0;
let tasks = 0;
const swarmOut = safe(() => {
  const out = execSync('npx @claude-flow/cli mcp exec --tool swarm_status 2>/dev/null', { encoding: 'utf8', timeout: 2000 });
  const m = out.match(/\{[\s\S]*\}/);
  return m ? JSON.parse(m[0]) : null;
}, null);
if (swarmOut && swarmOut.status) {
  swarmUp = swarmOut.status === 'running' || swarmOut.status === 'idle';
  agents = swarmOut.agentCount || 0;
  tasks = swarmOut.taskCount || 0;
}

// ── AgentDB health
let dbUp = false;
const dbOut = safe(() => {
  const out = execSync('npx @claude-flow/cli mcp exec --tool agentdb_health 2>/dev/null', { encoding: 'utf8', timeout: 2000 });
  const m = out.match(/\{[\s\S]*\}/);
  return m ? JSON.parse(m[0]) : null;
}, null);
if (dbOut && dbOut.available !== false) {
  dbUp = true;
}

// ── Dashboard — check registry for port
let dashUrl = '';
const regDir = path.join(require('os').homedir(), '.hex', 'registry');
const regFile = path.join(regDir, 'projects.json');
const regData = safe(() => JSON.parse(fs.readFileSync(regFile, 'utf8')), null);
if (regData && Array.isArray(regData)) {
  const match = regData.find(p => p.rootPath === process.cwd() || p.name === projectName);
  if (match && match.port) {
    // Quick check if dashboard is actually listening
    const listening = safe(() => {
      execSync(`curl -sf http://localhost:${match.port}/api/projects >/dev/null 2>&1`, { timeout: 1000 });
      return true;
    }, false);
    if (listening) dashUrl = `localhost:${match.port}`;
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
