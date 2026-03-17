import type { IGreetingInputPort } from '../../core/ports/greeting-port.js';

/** Primary adapter — accepts a name from CLI args and triggers the greeting use case. */
export class CliAdapter {
  constructor(private readonly greetingInput: IGreetingInputPort) {}

  async run(args: string[]): Promise<void> {
    const recipient = args[0] ?? 'World';
    await this.greetingInput.greet(recipient);
  }
}
