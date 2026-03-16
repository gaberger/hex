import type { ITodoQueryPort, ITodoCommandPort } from '../../core/ports/index.js';
import type { TodoData } from '../../core/domain/entities.js';
import { isValidPriority, isValidStatus, shortId } from '../../core/domain/value-objects.js';
import type { Priority, TodoStatus } from '../../core/domain/value-objects.js';

const RESET = '\x1b[0m';
const BOLD = '\x1b[1m';
const GREEN = '\x1b[32m';
const YELLOW = '\x1b[33m';
const RED = '\x1b[31m';
const CYAN = '\x1b[36m';
const DIM = '\x1b[2m';

const PRIORITY_COLOR: Record<Priority, string> = {
  high: RED,
  medium: YELLOW,
  low: DIM,
};

const STATUS_SYMBOL: Record<TodoData['status'], string> = {
  pending: '[ ]',
  'in-progress': '[~]',
  completed: '[x]',
};

function formatTodo(t: TodoData): string {
  const sym = STATUS_SYMBOL[t.status];
  const pc = PRIORITY_COLOR[t.priority];
  const id = `${DIM}${shortId(t.id)}${RESET}`;
  const title = t.status === 'completed' ? `${DIM}${t.title}${RESET}` : t.title;
  const pri = `${pc}${t.priority}${RESET}`;
  const tags = t.tags.length > 0 ? ` ${CYAN}[${t.tags.join(', ')}]${RESET}` : '';
  return `${sym} ${id} ${title} ${pri}${tags}`;
}

function parseArgs(args: string[]): { positional: string[]; flags: Record<string, string> } {
  const positional: string[] = [];
  const flags: Record<string, string> = {};
  for (let i = 0; i < args.length; i++) {
    if (args[i].startsWith('--') && i + 1 < args.length) {
      flags[args[i].slice(2)] = args[i + 1];
      i++;
    } else {
      positional.push(args[i]);
    }
  }
  return { positional, flags };
}

export class CliAdapter {
  constructor(
    private readonly queries: ITodoQueryPort,
    private readonly commands: ITodoCommandPort,
  ) {}

  async run(argv: string[]): Promise<void> {
    const [command, ...rest] = argv;
    try {
      switch (command) {
        case 'add': return await this.add(rest);
        case 'list': return await this.list(rest);
        case 'complete': return await this.complete(rest);
        case 'update': return await this.update(rest);
        case 'delete': return await this.remove(rest);
        case 'stats': return await this.stats();
        case 'serve': return; // handled by composition root
        default: return this.help();
      }
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      process.stdout.write(`${RED}Error:${RESET} ${msg}\n`);
      process.exitCode = 1;
    }
  }

  private async add(args: string[]): Promise<void> {
    const { positional, flags } = parseArgs(args);
    const title = positional[0];
    if (!title) {
      process.stdout.write(`${RED}Usage:${RESET} todo add "title" [--priority high] [--tags a,b]\n`);
      return;
    }
    const priority = flags.priority && isValidPriority(flags.priority) ? flags.priority : undefined;
    const tags = flags.tags ? flags.tags.split(',').map((t) => t.trim()) : undefined;
    const todo = await this.commands.create(title, priority, tags);
    process.stdout.write(`${GREEN}Created:${RESET} ${formatTodo(todo)}\n`);
  }

  private async list(args: string[]): Promise<void> {
    const { flags } = parseArgs(args);
    const status = flags.status && isValidStatus(flags.status) ? flags.status as TodoStatus : undefined;
    const priority = flags.priority && isValidPriority(flags.priority) ? flags.priority : undefined;
    const todos = await this.queries.filter(status, priority);
    if (todos.length === 0) {
      process.stdout.write(`${DIM}No todos found.${RESET}\n`);
      return;
    }
    for (const t of todos) {
      process.stdout.write(formatTodo(t) + '\n');
    }
  }

  private async complete(args: string[]): Promise<void> {
    const id = args[0];
    if (!id) { process.stdout.write(`${RED}Usage:${RESET} todo complete <id>\n`); return; }
    const resolved = await this.resolveId(id);
    if (!resolved) return;
    const todo = await this.commands.complete(resolved);
    process.stdout.write(`${GREEN}Completed:${RESET} ${formatTodo(todo)}\n`);
  }

  private async update(args: string[]): Promise<void> {
    const { positional, flags } = parseArgs(args);
    const id = positional[0];
    if (!id) { process.stdout.write(`${RED}Usage:${RESET} todo update <id> [--title ...] [--priority ...]\n`); return; }
    const resolved = await this.resolveId(id);
    if (!resolved) return;
    const changes: Record<string, unknown> = {};
    if (flags.title) changes.title = flags.title;
    if (flags.priority && isValidPriority(flags.priority)) changes.priority = flags.priority;
    if (flags.tags) changes.tags = flags.tags.split(',').map((t) => t.trim());
    const todo = await this.commands.update(resolved, changes);
    process.stdout.write(`${GREEN}Updated:${RESET} ${formatTodo(todo)}\n`);
  }

  private async remove(args: string[]): Promise<void> {
    const id = args[0];
    if (!id) { process.stdout.write(`${RED}Usage:${RESET} todo delete <id>\n`); return; }
    const resolved = await this.resolveId(id);
    if (!resolved) return;
    await this.commands.delete(resolved);
    process.stdout.write(`${GREEN}Deleted:${RESET} ${shortId(resolved)}\n`);
  }

  private async stats(): Promise<void> {
    const s = await this.queries.stats();
    process.stdout.write(
      `${BOLD}Todo Stats${RESET}\n` +
      `  Total:     ${s.total}\n` +
      `  Pending:   ${s.pending}\n` +
      `  Completed: ${s.completed}\n` +
      `  Rate:      ${Math.round(s.rate * 100)}%\n`,
    );
  }

  private async resolveId(partial: string): Promise<string | null> {
    const all = await this.queries.getAll();
    const matches = all.filter((t) => t.id.startsWith(partial));
    if (matches.length === 1) return matches[0].id;
    if (matches.length === 0) {
      process.stdout.write(`${RED}No todo found matching:${RESET} ${partial}\n`);
      return null;
    }
    process.stdout.write(`${RED}Ambiguous ID, matches:${RESET}\n`);
    matches.forEach((t) => process.stdout.write(`  ${shortId(t.id)} ${t.title}\n`));
    return null;
  }

  private help(): void {
    process.stdout.write(
      `${BOLD}Todo CLI${RESET} — Hexagonal Architecture Demo\n\n` +
      `  add    "title" [--priority high] [--tags a,b]\n` +
      `  list   [--status pending] [--priority high]\n` +
      `  complete <id>\n` +
      `  update <id> [--title ...] [--priority ...] [--tags ...]\n` +
      `  delete <id>\n` +
      `  stats\n` +
      `  serve  (start HTTP server on :3456)\n`,
    );
  }
}
