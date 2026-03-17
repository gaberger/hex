import { ConsoleOutputAdapter } from './adapters/secondary/console-output-adapter.js';
import { CliAdapter } from './adapters/primary/cli-adapter.js';
import { GreetUseCase } from './core/usecases/greet-usecase.js';

/**
 * Composition root — the ONLY file that imports adapters.
 * Wires adapters to ports via constructor injection.
 */
export function compose(): CliAdapter {
  const output = new ConsoleOutputAdapter();
  const useCase = new GreetUseCase(output);
  return new CliAdapter(useCase);
}
