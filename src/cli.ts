#!/usr/bin/env node
/**
 * CLI entry point -- referenced by package.json "bin".
 */

import { createAppContext } from './composition-root.js';
import { CLIAdapter } from './adapters/primary/cli-adapter.js';

const isDebug = process.argv.includes('--verbose') || process.env.HEX_DEBUG === '1';

try {
  const args = process.argv.slice(2);
  const autoConfirm = args.includes('--yes') || args.includes('-y');
  const filteredArgs = args.filter((a) => a !== '--yes' && a !== '-y' && a !== '--verbose');

  const ctx = await createAppContext(process.cwd());
  ctx.autoConfirm = autoConfirm;

  const cli = new CLIAdapter(ctx);
  const exitCode = await cli.run(filteredArgs);
  process.exit(exitCode);
} catch (err) {
  const message = err instanceof Error ? err.message : String(err);
  process.stderr.write(`Error: ${message}\n`);
  if (isDebug && err instanceof Error && err.stack) {
    process.stderr.write(`${err.stack}\n`);
  }
  process.exit(1);
}
