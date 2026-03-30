export interface CommandRequest {
  readonly id: string;
  readonly type: string;
  readonly payload: unknown;
  readonly timestamp: Date;
}

export interface CommandResponse {
  readonly requestId: string;
  readonly success: boolean;
  readonly data?: unknown;
  readonly error?: string;
  readonly timestamp: Date;
}

export interface QueryRequest {
  readonly id: string;
  readonly type: string;
  readonly parameters: Record<string, unknown>;
  readonly timestamp: Date;
}

export interface QueryResponse {
  readonly requestId: string;
  readonly success: boolean;
  readonly data?: unknown;
  readonly error?: string;
  readonly timestamp: Date;
}

export interface IncomingRequestHandler<TRequest, TResponse> {
  handle(request: TRequest): Promise<TResponse>;
}

export interface CommandHandler extends IncomingRequestHandler<CommandRequest, CommandResponse> {}

export interface QueryHandler extends IncomingRequestHandler<QueryRequest, QueryResponse> {}

export interface RequestValidator<T> {
  validate(request: T): Promise<ValidationResult>;
}

export interface ValidationResult {
  readonly isValid: boolean;
  readonly errors: string[];
}

export interface IncomingRequestPort {
  handleCommand(request: CommandRequest): Promise<CommandResponse>;
  handleQuery(request: QueryRequest): Promise<QueryResponse>;
  validateRequest<T>(request: T): Promise<ValidationResult>;
}