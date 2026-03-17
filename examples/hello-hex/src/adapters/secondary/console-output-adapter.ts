import type { IGreetingOutputPort } from '../../core/ports/greeting-port.js';
import type { Greeting } from '../../core/domain/greeting.js';

/** Secondary adapter — delivers greetings to stdout. */
export class ConsoleOutputAdapter implements IGreetingOutputPort {
  async deliver(greeting: Greeting): Promise<void> {
    console.log(greeting.message);
  }
}
