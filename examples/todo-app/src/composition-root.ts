import { TodoService } from './core/usecases/todo-service.js';
import { JsonStorageAdapter } from './adapters/secondary/json-storage.js';
import { ConsoleLoggerAdapter } from './adapters/secondary/console-logger.js';
import { CliAdapter } from './adapters/primary/cli-adapter.js';
import { HttpAdapter } from './adapters/primary/http-adapter.js';
import type { LogLevel } from './core/ports/logger.js';

export interface AppContext {
  cli: CliAdapter;
  http: HttpAdapter;
}

export function createApp(storageDir: string, logLevel: LogLevel = 'info'): AppContext {
  // Secondary adapters (driven)
  const storage = new JsonStorageAdapter(storageDir);
  const logger = new ConsoleLoggerAdapter(logLevel);

  // Use case (implements both query and command ports)
  const todoService = new TodoService(storage, logger);

  // Primary adapters (driving)
  const cli = new CliAdapter(todoService, todoService);
  const http = new HttpAdapter(todoService, todoService, logger);

  return { cli, http };
}
