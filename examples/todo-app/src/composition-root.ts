import { TodoService } from './core/usecases/todo-service.js';
import { JsonStorageAdapter } from './adapters/secondary/json-storage.js';
import { CliAdapter } from './adapters/primary/cli-adapter.js';
import { HttpAdapter } from './adapters/primary/http-adapter.js';

export interface AppContext {
  cli: CliAdapter;
  http: HttpAdapter;
}

export function createApp(storageDir: string): AppContext {
  // Secondary adapter (driven)
  const storage = new JsonStorageAdapter(storageDir);

  // Use case (implements both query and command ports)
  const todoService = new TodoService(storage);

  // Primary adapters (driving)
  const cli = new CliAdapter(todoService, todoService);
  const http = new HttpAdapter(todoService, todoService);

  return { cli, http };
}
