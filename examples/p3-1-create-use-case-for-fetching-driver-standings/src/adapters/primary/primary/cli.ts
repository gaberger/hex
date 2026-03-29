import { Command } from 'commander';

const program = new Command();

program
  .command('fetch-standings')
  .description('Fetch driver standings with pagination')
  .action(() => {
    // Placeholder action
  });

program.parse(process.argv);