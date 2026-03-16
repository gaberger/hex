#!/usr/bin/env bun
/**
 * CLI entry point -- referenced by package.json "bin".
 */

import { createAppContext } from './composition-root.js';
import { CLIAdapter } from './adapters/primary/cli-adapter.js';

const args = process.argv.slice(2);
const autoConfirm = args.includes('--yes') || args.includes('-y');
const filteredArgs = args.filter((a) => a !== '--yes' && a !== '-y');

const ctx = await createAppContext(process.cwd());
ctx.autoConfirm = autoConfirm;

const cli = new CLIAdapter(ctx);
const exitCode = await cli.run(filteredArgs);
process.exit(exitCode);
