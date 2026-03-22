#!/usr/bin/env node
/**
 * hex-hub auto-rebuild hook
 *
 * Triggered after edits to hex-hub/ source files.
 * Checks if any hex-hub Rust/HTML/CSS/JS files were modified,
 * and if so, runs `cargo build --release` in the background.
 *
 * Reads TOOL_INPUT from stdin to determine which file was edited.
 */
'use strict';

const { execFile } = require('child_process');
const { existsSync } = require('fs');
const { join } = require('path');

const projectDir = process.env.CLAUDE_PROJECT_DIR || process.cwd();

// Read tool input from stdin
let input = '';
process.stdin.setEncoding('utf-8');
process.stdin.on('data', (chunk) => { input += chunk; });
process.stdin.on('end', () => {
  try {
    const toolInput = JSON.parse(input);
    const filePath = toolInput.file_path || toolInput.filePath || '';

    // Only trigger for hex-hub source files
    if (!filePath.includes('hex-hub/')) {
      process.exit(0);
      return;
    }

    // Check if it's a relevant file type
    const relevant = /\.(rs|toml|html|css|js)$/.test(filePath);
    if (!relevant) {
      process.exit(0);
      return;
    }

    // Check that Cargo.toml exists
    const cargoToml = join(projectDir, 'Cargo.toml');
    if (!existsSync(cargoToml)) {
      process.exit(0);
      return;
    }

    // Trigger rebuild in the background (fire-and-forget)
    process.stderr.write('[hex-hub] Rebuilding after source change...\n');
    const child = execFile('cargo', ['build', '--release'], {
      cwd: projectDir,
      timeout: 120000,
    }, (err) => {
      if (err) {
        process.stderr.write(`[hex-hub] Rebuild failed: ${err.message}\n`);
      } else {
        process.stderr.write('[hex-hub] Rebuild complete.\n');
      }
    });
    child.unref();

    // Don't block the hook — exit immediately
    process.exit(0);
  } catch {
    // Not JSON or missing fields — skip
    process.exit(0);
  }
});
