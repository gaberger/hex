#!/usr/bin/env bun
/**
 * Multi-Instance Coordination Test Script
 *
 * Tests the coordination session cleanup feature (ADR-023) by:
 * 1. Starting hex-hub daemon
 * 2. Registering multiple coordination instances
 * 3. Simulating heartbeats, stale sessions, and dead PIDs
 * 4. Verifying cleanup behavior
 *
 * Usage:
 *   bun scripts/test-coordination.ts
 */

import { spawn } from 'node:child_process';

const HUB_URL = 'http://localhost:5555';
const PROJECT_ID = 'test-coordination-' + Date.now();

interface InstanceInfo {
  instanceId: string;
  pid: number;
  registered: string;
}

// ── Helper Functions ─────────────────────────────────────

async function post(path: string, body: unknown): Promise<unknown> {
  const res = await fetch(`${HUB_URL}${path}`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  if (!res.ok) {
    throw new Error(`POST ${path} failed: ${res.status} ${res.statusText}`);
  }
  return res.json();
}

async function get(path: string): Promise<unknown> {
  const res = await fetch(`${HUB_URL}${path}`);
  if (!res.ok) {
    throw new Error(`GET ${path} failed: ${res.status} ${res.statusText}`);
  }
  return res.json();
}

async function registerInstance(label: string): Promise<InstanceInfo> {
  const result = (await post('/api/coordination/instance/register', {
    projectId: PROJECT_ID,
    pid: process.pid,
    sessionLabel: label,
  })) as { instanceId: string };

  return {
    instanceId: result.instanceId,
    pid: process.pid,
    registered: new Date().toISOString(),
  };
}

async function heartbeat(instanceId: string, state?: { agents?: number; tasks?: number }): Promise<void> {
  await post('/api/coordination/instance/heartbeat', {
    instanceId: instanceId,
    projectId: PROJECT_ID,
    agentCount: state?.agents,
    activeTaskCount: state?.tasks,
  });
}

async function listInstances(): Promise<unknown[]> {
  const result = (await get(`/api/coordination/instances?projectId=${PROJECT_ID}`)) as unknown[];
  return result;
}

async function manualCleanup(): Promise<{ removed: number }> {
  const result = (await post('/api/coordination/cleanup', {})) as { removed: number };
  return result;
}

async function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

// ── Test Scenarios ───────────────────────────────────────

async function testNormalHeartbeat(): Promise<void> {
  console.log('\n📍 Test 1: Normal heartbeat (instance stays alive)');

  const instance = await registerInstance('test-normal-heartbeat');
  console.log(`  ✓ Registered instance: ${instance.instanceId.slice(0, 8)}`);

  // Send heartbeats for 30 seconds
  for (let i = 0; i < 6; i++) {
    await sleep(5000);
    await heartbeat(instance.instanceId, { agents: 2, tasks: 3 });
    console.log(`  ✓ Heartbeat ${i + 1}/6 sent`);
  }

  const instances = await listInstances();
  const found = instances.find((i: any) => i.instanceId === instance.instanceId);

  if (found) {
    console.log('  ✅ PASS: Instance still registered after heartbeats');
  } else {
    console.log('  ❌ FAIL: Instance was incorrectly removed');
  }
}

async function testStaleSession(): Promise<void> {
  console.log('\n📍 Test 2: Stale session (no heartbeat for 60s)');

  const instance = await registerInstance('test-stale-session');
  console.log(`  ✓ Registered instance: ${instance.instanceId.slice(0, 8)}`);
  console.log('  ⏳ Waiting 70 seconds without heartbeat...');

  await sleep(70_000); // Wait longer than stale threshold (60s)

  // Manual cleanup should remove it
  const result = await manualCleanup();
  console.log(`  ✓ Manual cleanup removed ${result.removed} session(s)`);

  const instances = await listInstances();
  const found = instances.find((i: any) => i.instanceId === instance.instanceId);

  if (!found) {
    console.log('  ✅ PASS: Stale instance was removed');
  } else {
    console.log('  ❌ FAIL: Stale instance still registered');
  }
}

async function testDeadPID(): Promise<void> {
  console.log('\n📍 Test 3: Dead PID (process terminated)');

  // Spawn a short-lived child process
  const child = spawn('sleep', ['1'], { detached: false });
  const childPid = child.pid!;

  const instance = await registerInstance('test-dead-pid');
  console.log(`  ✓ Registered instance with PID: ${childPid}`);

  // Override instance PID to the child process
  // (In real usage, each instance would have its own PID)
  await post('/api/coordination/instance/register', {
    projectId: PROJECT_ID,
    pid: childPid,
    session_label: 'test-dead-pid-child',
  });

  // Wait for child to exit
  await new Promise((resolve) => child.on('exit', resolve));
  console.log('  ✓ Child process exited');

  // Wait a bit for cleanup cron to run (runs every 60s)
  console.log('  ⏳ Waiting for cleanup cron to detect dead PID...');
  await sleep(65_000);

  const instances = await listInstances();
  const found = instances.find((i: any) => i.pid === childPid);

  if (!found) {
    console.log('  ✅ PASS: Dead PID instance was removed');
  } else {
    console.log('  ❌ FAIL: Dead PID instance still registered');
  }
}

async function testManualCleanup(): Promise<void> {
  console.log('\n📍 Test 4: Manual cleanup button');

  // Register 3 instances, only heartbeat 1
  const instances = await Promise.all([
    registerInstance('manual-cleanup-active'),
    registerInstance('manual-cleanup-stale-1'),
    registerInstance('manual-cleanup-stale-2'),
  ]);
  console.log(`  ✓ Registered 3 instances`);

  // Only heartbeat the first one
  await heartbeat(instances[0].instanceId);
  console.log('  ✓ Sent heartbeat for instance 1 only');

  // Wait for others to become stale
  console.log('  ⏳ Waiting 70 seconds for instances 2-3 to become stale...');
  await sleep(70_000);

  // Continue heartbeating the active one
  await heartbeat(instances[0].instanceId);

  // Manual cleanup
  const result = await manualCleanup();
  console.log(`  ✓ Manual cleanup removed ${result.removed} session(s)`);

  const remaining = await listInstances();
  const activeFound = remaining.find((i: any) => i.instanceId === instances[0].instanceId);
  const stale1Found = remaining.find((i: any) => i.instanceId === instances[1].instanceId);
  const stale2Found = remaining.find((i: any) => i.instanceId === instances[2].instanceId);

  if (activeFound && !stale1Found && !stale2Found) {
    console.log('  ✅ PASS: Active instance kept, stale instances removed');
  } else {
    console.log(`  ❌ FAIL: Unexpected state (active=${!!activeFound}, stale1=${!!stale1Found}, stale2=${!!stale2Found})`);
  }
}

async function testCoordinationState(): Promise<void> {
  console.log('\n📍 Test 5: Coordination state (locks, claims, activities)');

  const instance = await registerInstance('test-coordination-state');
  console.log(`  ✓ Registered instance: ${instance.instanceId.slice(0, 8)}`);

  // Acquire a worktree lock
  await post('/api/coordination/worktree/lock', {
    instanceId: instance.instanceId,
    projectId: PROJECT_ID,
    feature: 'test-feature',
    layer: 'adapters/primary',
    ttlSecs: 300,
  });
  console.log('  ✓ Acquired worktree lock');

  // Claim a task
  await post('/api/coordination/task/claim', {
    instanceId: instance.instanceId,
    taskId: 'test-task-123',
  });
  console.log('  ✓ Claimed task');

  // Publish activity
  await post('/api/coordination/activity', {
    instanceId: instance.instanceId,
    projectId: PROJECT_ID,
    action: 'test-action',
    details: { test: true },
  });
  console.log('  ✓ Published activity');

  // Verify state
  const [locks, claims, activities] = await Promise.all([
    get(`/api/coordination/worktree/locks?projectId=${PROJECT_ID}`),
    get(`/api/coordination/tasks?projectId=${PROJECT_ID}`),
    get(`/api/coordination/activities?projectId=${PROJECT_ID}&limit=10`),
  ]);

  const hasLock = (locks as any[]).some((l) => l.instanceId === instance.instanceId);
  const hasClaim = (claims as any[]).some((c) => c.instanceId === instance.instanceId);
  const hasActivity = (activities as any[]).some((a) => a.instanceId === instance.instanceId);

  if (hasLock && hasClaim && hasActivity) {
    console.log('  ✅ PASS: Coordination state verified');
  } else {
    console.log(`  ❌ FAIL: Missing state (lock=${hasLock}, claim=${hasClaim}, activity=${hasActivity})`);
  }
}

// ── Main ─────────────────────────────────────────────────

async function main() {
  console.log('🧪 Multi-Instance Coordination Test Suite');
  console.log('━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━');
  console.log(`Hub: ${HUB_URL}`);
  console.log(`Project: ${PROJECT_ID}`);

  // Check if hub is running
  try {
    await get('/api/version');
  } catch {
    console.error('\n❌ hex-nexus is not running. Start it with: hex daemon start');
    process.exit(1);
  }

  console.log('\nRunning tests...\n');

  try {
    await testNormalHeartbeat();
    await testManualCleanup();
    await testCoordinationState();

    // These tests require long waits — skip by default
    // await testStaleSession();
    // await testDeadPID();

    console.log('\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━');
    console.log('✅ Test suite complete!');
    console.log('\nNote: Stale session and dead PID tests skipped (require 60s+ waits)');
    console.log('To run full suite including slow tests, uncomment them in the script.');
  } catch (err) {
    console.error('\n❌ Test failed:', err);
    process.exit(1);
  }
}

main();
