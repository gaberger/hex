#!/usr/bin/env node
/**
 * hex-agent session registration hook
 *
 * Registers the Claude Code session as a hex-agent with SpacetimeDB
 * via the hex nexus REST API on SessionStart, and deregisters on SessionEnd.
 *
 * Usage (from Claude Code hooks):
 *   node agent-register.cjs register    — SessionStart
 *   node agent-register.cjs deregister  — SessionEnd
 *
 * Environment (set by Claude Code):
 *   CLAUDE_SESSION_ID  — current session ID
 *   CLAUDE_PROJECT_DIR — project root
 *   CLAUDE_MODEL       — model in use (if available)
 */

'use strict';

const http = require('http');
const path = require('path');
const fs = require('fs');
const os = require('os');

const HUB_PORT = 5555;
const TIMEOUT_MS = 3000;

// Persist the agentId between register/deregister across the session
const STATE_DIR = path.join(os.homedir(), '.hex', 'sessions');
const sessionId = process.env.CLAUDE_SESSION_ID || 'unknown';
const stateFile = path.join(STATE_DIR, `agent-${sessionId}.json`);

// Read auth token from hub lock file
let authToken = '';
try {
  const lockPath = path.join(os.homedir(), '.hex', 'daemon', 'hub.lock');
  const lock = JSON.parse(fs.readFileSync(lockPath, 'utf-8'));
  authToken = lock.token || '';
} catch { /* no lock file — push without auth */ }

const action = process.argv[2] || 'register';

if (action === 'register') {
  register();
} else if (action === 'deregister') {
  deregister();
} else {
  process.stderr.write(`agent-register: unknown action "${action}"\n`);
  process.exit(1);
}

function register() {
  const projectDir = process.env.HEX_PROJECT_ROOT || process.env.CLAUDE_PROJECT_DIR || process.cwd();
  const model = process.env.CLAUDE_MODEL || '';

  const payload = JSON.stringify({
    host: os.hostname(),
    name: `claude-code-${sessionId.slice(0, 8)}`,
    project_dir: projectDir,
    model: model,
    session_id: sessionId,
  });

  const headers = {
    'Content-Type': 'application/json',
    'Content-Length': Buffer.byteLength(payload),
  };
  if (authToken) headers['Authorization'] = 'Bearer ' + authToken;

  const req = http.request({
    hostname: '127.0.0.1',
    port: HUB_PORT,
    path: '/api/agents/connect',
    method: 'POST',
    headers,
    timeout: TIMEOUT_MS,
  }, (res) => {
    let body = '';
    res.on('data', (chunk) => { body += chunk; });
    res.on('end', () => {
      try {
        const result = JSON.parse(body);
        if (result.agentId) {
          // Persist agentId so deregister can find it
          fs.mkdirSync(STATE_DIR, { recursive: true });
          fs.writeFileSync(stateFile, JSON.stringify({
            agentId: result.agentId,
            sessionId,
            registeredAt: new Date().toISOString(),
          }));
          // Print for hook output (visible in session context)
          process.stdout.write(`hex-agent registered: ${result.agentId}\n`);
        }
      } catch { /* ignore parse errors */ }
    });
  });

  req.on('error', () => {
    // Nexus not running — silently skip (non-blocking)
  });
  req.on('timeout', () => req.destroy());
  req.end(payload);
}

function deregister() {
  let agentId;
  try {
    const state = JSON.parse(fs.readFileSync(stateFile, 'utf-8'));
    agentId = state.agentId;
  } catch {
    // No state file — never registered, nothing to do
    return;
  }

  const payload = JSON.stringify({ agentId });

  const headers = {
    'Content-Type': 'application/json',
    'Content-Length': Buffer.byteLength(payload),
  };
  if (authToken) headers['Authorization'] = 'Bearer ' + authToken;

  const req = http.request({
    hostname: '127.0.0.1',
    port: HUB_PORT,
    path: '/api/agents/disconnect',
    method: 'POST',
    headers,
    timeout: TIMEOUT_MS,
  }, (res) => {
    res.resume();
    res.on('end', () => {
      // Clean up state file
      try { fs.unlinkSync(stateFile); } catch { /* ignore */ }
    });
  });

  req.on('error', () => {
    // Nexus not running — clean up state file anyway
    try { fs.unlinkSync(stateFile); } catch { /* ignore */ }
  });
  req.on('timeout', () => req.destroy());
  req.end(payload);
}

// Read stdin but don't block
process.stdin.resume();
process.stdin.on('data', () => {});
process.stdin.on('end', () => {});

// Exit quickly
setTimeout(() => process.exit(0), TIMEOUT_MS + 500);
