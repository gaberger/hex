import{ CliCommand } from '../../ports/cli-command.js';

export function cliCommand(): string {
  const command: CliCommand = {
    name: 'f1-data',
    description: 'Fetch Formula 1 data',
    execute: () => 'Data fetched successfully'
  };
  return command.execute();
}