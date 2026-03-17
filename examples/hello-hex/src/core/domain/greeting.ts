/** Domain value object — a greeting message. */
export interface Greeting {
  readonly recipient: string;
  readonly message: string;
  readonly timestamp: Date;
}

/** Create a Greeting value object. Pure function, no side effects. */
export function createGreeting(recipient: string): Greeting {
  return {
    recipient,
    message: `Hello, ${recipient}! Welcome to hex.`,
    timestamp: new Date(),
  };
}
