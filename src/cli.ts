#!/usr/bin/env bun
/**
 * CLI entry point -- referenced by package.json "bin".
 */

import { createAppContext } from './composition-root.js';
import { CLIAdapter } from './adapters/primary/cli-adapter.js';

const ctx = await createAppContext(process.cwd());
const cli = new CLIAdapter(ctx);
const exitCode = await cli.run(process.argv.slice(2));
process.exit(exitCode);
