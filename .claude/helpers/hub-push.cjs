#!/usr/bin/env node
/**
 * hex-hub event push hook
 *
 * Pushes Claude Code tool-use events to the hex-hub dashboard (port 5555).
 * Lightweight, non-blocking — failures are silently ignored so they never
 * break the agent's workflow.
 *
 * Usage (from Claude Code hooks):
 *   node hub-push.cjs <event-type>
 *
 * Environment (set by Claude Code):
 *   CLAUDE_TOOL_NAME     — tool being used (Read, Write, Bash, etc.)
 *   CLAUDE_FILE_PATH     — file path (for Read/Write/Edit)
 *   CLAUDE_SESSION_ID    — current session ID
 *   CLAUDE_PROJECT_DIR   — project root
 *
 * Reads stdin for the tool input JSON (Claude Code pipes it).
 */

'use strict';

const http = require('http');
const path = require('path');

const HUB_PORT = 5555;
const TIMEOUT_MS = 2000;

const eventType = process.argv[2] || 'tool-use';

// Build event payload from environment
const projectDir = process.env.CLAUDE_PROJECT_DIR || process.cwd();
const projectName = path.basename(projectDir);
const toolName = process.env.CLAUDE_TOOL_NAME || 'unknown';
const filePath = process.env.CLAUDE_FILE_PATH || '';
const sessionId = process.env.CLAUDE_SESSION_ID || '';

// Determine project ID (matches hex-hub's make_project_id in state.rs)
// Uses unsigned 32-bit wrapping arithmetic: ((h << 5) - h + c) as u32
function makeProjectId(rootPath) {
  const name = path.basename(rootPath);
  let hash = 0;
  for (let i = 0; i < rootPath.length; i++) {
    // Unsigned 32-bit wrapping: (h << 5) wrapping_sub h wrapping_add c
    hash = (((hash << 5) >>> 0) - hash + rootPath.charCodeAt(i)) >>> 0;
  }
  return `${name}-${hash.toString(36)}`;
}

const projectId = makeProjectId(projectDir);

// Classify the event for dashboard display
function classifyEvent(tool, file) {
  if (tool === 'Write' || tool === 'Edit' || tool === 'MultiEdit') {
    const layer = file.includes('/domain/') ? 'domain'
      : file.includes('/ports/') ? 'port'
      : file.includes('/usecases/') ? 'usecase'
      : file.includes('/adapters/primary/') ? 'primary-adapter'
      : file.includes('/adapters/secondary/') ? 'secondary-adapter'
      : file.includes('/test') ? 'test'
      : 'other';
    return { action: 'edit', layer, file: path.basename(file) };
  }
  if (tool === 'Read') return { action: 'read', file: path.basename(file) };
  if (tool === 'Bash') return { action: 'command' };
  if (tool === 'Agent') return { action: 'agent-spawn' };
  return { action: tool.toLowerCase() };
}

const detail = classifyEvent(toolName, filePath);

const payload = JSON.stringify({
  projectId,
  event: `agent-${eventType}`,
  data: {
    tool: toolName,
    ...detail,
    filePath: filePath ? path.relative(projectDir, filePath) : undefined,
    sessionId,
    timestamp: Date.now(),
  },
});

// Fire-and-forget POST to hub
const req = http.request({
  hostname: '127.0.0.1',
  port: HUB_PORT,
  path: '/api/event',
  method: 'POST',
  headers: { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) },
  timeout: TIMEOUT_MS,
}, (res) => { res.resume(); }); // drain response

req.on('error', () => {}); // silent
req.on('timeout', () => req.destroy());
req.end(payload);

// Read stdin but don't block — just drain it
process.stdin.resume();
process.stdin.on('data', () => {});
process.stdin.on('end', () => {});

// Exit quickly — don't hold up the agent
setTimeout(() => process.exit(0), TIMEOUT_MS + 100);
