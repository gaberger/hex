export interface CommandSession {
  readonly id: SessionId;
  readonly startTime: Date;
  readonly endTime?: Date;
  readonly status: SessionStatus;
  readonly commands: readonly CommandExecution[];
}

export class SessionId {
  private constructor(private readonly value: string) {}

  static create(): SessionId {
    const timestamp = Date.now().toString(36);
    const random = Math.random().toString(36).substring(2);
    return new SessionId(`session_${timestamp}_${random}`);
  }

  static fromString(value: string): SessionId {
    if (!value || typeof value !== 'string' || value.trim().length === 0) {
      throw new Error('SessionId cannot be empty');
    }
    return new SessionId(value);
  }

  toString(): string {
    return this.value;
  }

  equals(other: SessionId): boolean {
    return this.value === other.value;
  }
}

export enum SessionStatus {
  ACTIVE = 'active',
  COMPLETED = 'completed',
  FAILED = 'failed',
  CANCELLED = 'cancelled'
}

export interface CommandExecution {
  readonly id: CommandExecutionId;
  readonly command: string;
  readonly arguments: readonly string[];
  readonly workingDirectory: string;
  readonly startTime: Date;
  readonly endTime?: Date;
  readonly exitCode?: number;
  readonly output: CommandOutput;
}

export class CommandExecutionId {
  private constructor(private readonly value: string) {}

  static create(): CommandExecutionId {
    const timestamp = Date.now().toString(36);
    const random = Math.random().toString(36).substring(2);
    return new CommandExecutionId(`cmd_${timestamp}_${random}`);
  }

  static fromString(value: string): CommandExecutionId {
    if (!value || typeof value !== 'string' || value.trim().length === 0) {
      throw new Error('CommandExecutionId cannot be empty');
    }
    return new CommandExecutionId(value);
  }

  toString(): string {
    return this.value;
  }

  equals(other: CommandExecutionId): boolean {
    return this.value === other.value;
  }
}

export interface CommandOutput {
  readonly stdout: string;
  readonly stderr: string;
}

export class WorkingDirectory {
  private constructor(private readonly value: string) {}

  static fromString(path: string): WorkingDirectory {
    if (!path || typeof path !== 'string' || path.trim().length === 0) {
      throw new Error('WorkingDirectory path cannot be empty');
    }
    return new WorkingDirectory(path);
  }

  toString(): string {
    return this.value;
  }

  equals(other: WorkingDirectory): boolean {
    return this.value === other.value;
  }
}

export class CommandLine {
  private constructor(
    private readonly command: string,
    private readonly args: readonly string[]
  ) {}

  static create(command: string, args: string[] = []): CommandLine {
    if (!command || typeof command !== 'string' || command.trim().length === 0) {
      throw new Error('Command cannot be empty');
    }
    return new CommandLine(command.trim(), Object.freeze([...args]));
  }

  getCommand(): string {
    return this.command;
  }

  getArguments(): readonly string[] {
    return this.args;
  }

  toString(): string {
    if (this.args.length === 0) {
      return this.command;
    }
    const escapedArgs = this.args.map(arg => 
      arg.includes(' ') ? `"${arg}"` : arg
    );
    return `${this.command} ${escapedArgs.join(' ')}`;
  }

  equals(other: CommandLine): boolean {
    return this.command === other.getCommand() && 
           this.args.length === other.getArguments().length &&
           this.args.every((arg, index) => arg === other.getArguments()[index]);
  }
}