import { createGreeting, type Greeting } from '../domain/greeting.js';
import type { IGreetingInputPort, IGreetingOutputPort } from '../ports/greeting-port.js';

/** Use case: greet a recipient and deliver the greeting via the output port. */
export class GreetUseCase implements IGreetingInputPort {
  constructor(private readonly output: IGreetingOutputPort) {}

  async greet(recipient: string): Promise<Greeting> {
    const greeting = createGreeting(recipient);
    await this.output.deliver(greeting);
    return greeting;
  }
}
