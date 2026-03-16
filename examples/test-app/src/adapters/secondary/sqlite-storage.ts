// Secondary adapter — implements IFavoritesStore via SQLite
// Imports only from ports
import { Database } from "bun:sqlite";
import type { IFavoritesStore } from "../../core/ports/index.js";
import type { FavoriteCity } from "../../core/domain/index.js";

export class SqliteFavoritesStore implements IFavoritesStore {
  private db: Database;

  constructor(dbPath: string) {
    this.db = new Database(dbPath);
    this.db.run(`
      CREATE TABLE IF NOT EXISTS favorites (
        id TEXT PRIMARY KEY,
        city TEXT NOT NULL,
        country TEXT NOT NULL,
        added_at INTEGER NOT NULL
      )
    `);
  }

  async list(): Promise<FavoriteCity[]> {
    const rows = this.db
      .query("SELECT id, city, country, added_at FROM favorites ORDER BY added_at DESC")
      .all() as Array<{ id: string; city: string; country: string; added_at: number }>;

    return rows.map((r) => ({
      id: r.id,
      city: r.city,
      country: r.country,
      addedAt: r.added_at,
    }));
  }

  async add(favorite: FavoriteCity): Promise<void> {
    this.db
      .query("INSERT OR REPLACE INTO favorites (id, city, country, added_at) VALUES (?, ?, ?, ?)")
      .run(favorite.id, favorite.city, favorite.country, favorite.addedAt);
  }

  async remove(id: string): Promise<void> {
    this.db.query("DELETE FROM favorites WHERE id = ?").run(id);
  }

  close(): void {
    this.db.close();
  }
}
