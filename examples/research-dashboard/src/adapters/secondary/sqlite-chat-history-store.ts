import { Database } from "bun:sqlite";
import { mkdirSync } from "node:fs";
import { dirname } from "node:path";
import type { ChatAnswer, ChatSource } from "../../core/domain/entities.js";
import type { IChatHistoryStore } from "../../core/ports/index.js";

interface Row {
  question: string;
  answer: string;
  sources: string;
  answered_at: string;
}

export class SqliteChatHistoryStore implements IChatHistoryStore {
  private readonly db: Database;

  constructor(filePath: string) {
    mkdirSync(dirname(filePath), { recursive: true });
    this.db = new Database(filePath);
    this.db.run(`
      CREATE TABLE IF NOT EXISTS chat_history (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        question TEXT NOT NULL,
        answer TEXT NOT NULL,
        sources TEXT NOT NULL,
        answered_at TEXT NOT NULL
      )
    `);
  }

  async append(entry: ChatAnswer): Promise<void> {
    this.db
      .query("INSERT INTO chat_history (question, answer, sources, answered_at) VALUES (?, ?, ?, ?)")
      .run(entry.question, entry.answer, JSON.stringify(entry.sources), entry.answeredAt);
  }

  async list(): Promise<ChatAnswer[]> {
    const rows = this.db
      .query("SELECT question, answer, sources, answered_at FROM chat_history ORDER BY id ASC")
      .all() as Row[];
    return rows.map((r) => ({
      question: r.question,
      answer: r.answer,
      sources: JSON.parse(r.sources) as ChatSource[],
      answeredAt: r.answered_at,
    }));
  }
}
