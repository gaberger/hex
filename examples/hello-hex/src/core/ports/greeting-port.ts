import type { Greeting } from '../domain/greeting.js';

/** Output port — how greetings are delivered to the outside world. */
export interface IGreetingOutputPort {
  deliver(greeting: Greeting): Promise<void>;
}

/** Input port — how the application receives greeting requests. */
export interface IGreetingInputPort {
  greet(recipient: string): Promise<Greeting>;
}
