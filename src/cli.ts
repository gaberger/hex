#!/usr/bin/env node
/**
 * CLI entry point -- referenced by package.json "bin".
 */

import { createAppContext } from './composition-root.js';
import { CLIAdapter } from './adapters/primary/cli-adapter.js';

/** Prompt for vault password from TTY with echo disabled. */
function readVaultPassword(): Promise<string> {
  return new Promise<string>((resolve, reject) => {
    if (!process.stdin.isTTY) {
      reject(new Error('no TTY'));
      return;
    }
    process.stderr.write('Vault password: ');
    if (process.stdin.setRawMode) process.stdin.setRawMode(true);
    process.stdin.resume();
    let input = '';
    const onData = (chunk: Buffer) => {
      const char = chunk.toString();
      if (char === '\n' || char === '\r' || char === '\u0004') {
        process.stderr.write('\n');
        if (process.stdin.setRawMode) process.stdin.setRawMode(false);
        process.stdin.pause();
        process.stdin.removeListener('data', onData);
        resolve(input);
      } else if (char === '\u0003') {
        // Ctrl-C
        if (process.stdin.setRawMode) process.stdin.setRawMode(false);
        process.stdin.pause();
        process.stdin.removeListener('data', onData);
        reject(new Error('cancelled'));
      } else if (char === '\u007F' || char === '\b') {
        input = input.slice(0, -1);
      } else {
        input += char;
      }
    };
    process.stdin.on('data', onData);
  });
}

const isDebug = process.argv.includes('--verbose') || process.env.HEX_DEBUG === '1';

try {
  const args = process.argv.slice(2);
  const autoConfirm = args.includes('--yes') || args.includes('-y');
  const filteredArgs = args.filter((a) => a !== '--yes' && a !== '-y' && a !== '--verbose');

  // Commands that need LLM access benefit from vault password prompting
  const llmCommands = ['orchestrate', 'generate', 'plan', 'compare', 'validate'];
  const needsLLM = llmCommands.includes(filteredArgs[0]);

  const ctx = await createAppContext(process.cwd(), {
    getVaultPassword: needsLLM ? readVaultPassword : undefined,
  });
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
