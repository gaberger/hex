#!/usr/bin/env node
/**
 * hex status line — single colorful line showing framework status
 *
 * Reads .hex/status.json (written by composition root on startup)
 * and .hex/project.json for identity. Fast, no child processes.
 */
const fs = require('fs');
const path = require('path');
const { execSync } = require('child_process');

// ANSI colors
const R = '\x1b[0m';   // reset
const B = '\x1b[1m';   // bold
const D = '\x1b[2m';   // dim
const GRN = '\x1b[32m';
const YLW = '\x1b[33m';
const RED = '\x1b[31m';
const CYN = '\x1b[36m';
const MAG = '\x1b[35m';
const BLU = '\x1b[34m';
const WHT = '\x1b[37m';

function safe(fn, fb) { try { return fn(); } catch { return fb; } }

const cwd = process.cwd();
const hexDir = path.join(cwd, '.hex');

// ── Git branch
const branch = safe(() => execSync('git branch --show-current 2>/dev/null', { encoding: 'utf8' }).trim(), '?');

// ── Project identity from .hex/project.json
const projData = safe(() => JSON.parse(fs.readFileSync(path.join(hexDir, 'project.json'), 'utf8')), null);
const projectName = (projData && projData.name) || path.basename(cwd);
const projectId = (projData && projData.id) || '';

// ── Runtime status from .hex/status.json (written by composition root)
const statusData = safe(() => JSON.parse(fs.readFileSync(path.join(hexDir, 'status.json'), 'utf8')), null);
const swarmUp = statusData ? !!statusData.swarm : false;
const agentdbUp = statusData ? !!statusData.agentdb : false;
const dashUrl = statusData ? (statusData.dashboard || '') : '';

// ── Fallback: check claude-flow runtime if no status.json
const cfMetrics = path.join(require('os').homedir(), '.claude-flow', 'metrics');
const cfAlive = safe(() => fs.existsSync(cfMetrics) && (Date.now() - fs.statSync(cfMetrics).mtimeMs) < 300000, false);
const swarmShow = swarmUp || cfAlive;
const dbShow = agentdbUp || cfAlive;

// ── Health score
const score = safe(() => fs.readFileSync(path.join(hexDir, 'last-score.txt'), 'utf8').trim(), '');

// ── Hex MCP connected?
const mcpJson = safe(() => JSON.parse(fs.readFileSync(path.join(cwd, '.mcp.json'), 'utf8')), null);
const hexMcp = !!(mcpJson && mcpJson.mcpServers && mcpJson.mcpServers.hex);

// ── Build single line
const dot = (ok) => ok ? `${GRN}●${R}` : `${D}○${R}`;
const scoreCol = !score ? '' : parseInt(score) >= 80 ? GRN : parseInt(score) >= 50 ? YLW : RED;
const idShort = projectId ? projectId.slice(0, 8) : '';

const parts = [
  `${B}${MAG}⬡ hex${R}`,
  `${CYN}${projectName}${R}${idShort ? `${D}:${R}${idShort}` : ''}`,
  `${D}⎇${R}${WHT}${branch}${R}`,
  `${dot(swarmShow)}${D}swarm${R}`,
  `${dot(dbShow)}${D}db${R}`,
  dashUrl ? `${GRN}◉${R}\x1b]8;;http://${dashUrl}\x07${BLU}${dashUrl}${R}\x1b]8;;\x07` : `${dot(false)}${D}dash${R}`,
  hexMcp ? `${GRN}◉${R}${D}mcp${R}` : '',
  score ? `${scoreCol}${score}${R}${D}/100${R}` : '',
].filter(Boolean);

// Pad past column 25 to avoid Claude Code's collision zone
process.stdout.write('                          ' + parts.join(`${D} │ ${R}`));
