/**
 * useCommands.ts — Hook that provides the command list for the CommandPalette.
 *
 * Wraps the commands store in a reactive accessor, returning all commands
 * (including dynamic entity-based ones) grouped by category.
 */
import { createMemo } from "solid-js";
import {
  getAllCommandsWithEntities,
  searchCommands,
  type Command,
  type CommandCategory,
} from "../stores/commands";

export type { Command, CommandCategory };

export interface CommandGroup {
  category: CommandCategory;
  commands: Command[];
}

const CATEGORY_ORDER: CommandCategory[] = [
  "navigation",
  "project",
  "agent",
  "swarm",
  "analysis",
  "inference",
  "session",
  "view",
  "settings",
];

/**
 * Returns all available commands and helpers for searching/grouping them.
 */
export function useCommands() {
  const commands = createMemo(() => getAllCommandsWithEntities());

  const grouped = createMemo((): CommandGroup[] => {
    const all = commands();
    const byCategory = new Map<CommandCategory, Command[]>();

    for (const cmd of all) {
      const list = byCategory.get(cmd.category) ?? [];
      list.push(cmd);
      byCategory.set(cmd.category, list);
    }

    return CATEGORY_ORDER
      .filter((cat) => byCategory.has(cat))
      .map((cat) => ({ category: cat, commands: byCategory.get(cat)! }));
  });

  const search = (query: string) => searchCommands(query);

  return { commands, grouped, search };
}
