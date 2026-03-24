import { EventStore } from '../../ports/EventStore.js';
import { Event } from '../../domain/Event.js';
import { EventEnvelope } from '../../domain/EventEnvelope.js';
import { StreamName } from '../../domain/StreamName.js';
import { PgPool } from './PgPool.js';

export class PostgresEventStore implements EventStore {
  private readonly pool: PgPool;

  constructor(pool: PgPool) {
    this.pool = pool;
  }

  async appendToStream(streamName: StreamName, events: Event[]): Promise<void> {
    const client = await this.pool.connect();
    try {
      await client.query('BEGIN');
      const streamId = await this.getStreamId(client, streamName);
      const version = await this.getStreamVersion(client, streamId);
      if (version + events.length > Number.MAX_SAFE_INTEGER) {
        throw new Error('Stream version would exceed maximum safe integer');
      }
      for (const [index, event] of events.entries()) {
        await this.persistEvent(client, streamId, version + index + 1, event);
      }
      await client.query('COMMIT');
    } catch (error) {
      await client.query('ROLLBACK');
      throw error;
    } finally {
      client.release();
    }
  }

  async readFromStream(streamName: StreamName): Promise<EventEnvelope[]> {
    const client = await this.pool.connect();
    try {
      const streamId = await this.getStreamId(client, streamName);
      return await this.getEvents(client, streamId);
    } finally {
      client.release();
    }
  }

  private async getStreamId(client: any, streamName: StreamName): Promise<number> {
    const result = await client.query(
      `SELECT id FROM event_streams WHERE name = $1`,
      [streamName.value]
    );
    if (result.rows.length === 0) {
      const insertResult = await client.query(
        `INSERT INTO event_streams (name) VALUES ($1) RETURNING id`,
        [streamName.value]
      );
      return insertResult.rows[0].id;
    }
    return result.rows[0].id;
  }

  private async getStreamVersion(client: any, streamId: number): Promise<number> {
    const result = await client.query(
      `SELECT COALESCE(MAX(stream_version), 0) AS version FROM events WHERE stream_id = $1`,
      [streamId]
    );
    return result.rows[0].version;
  }

  private async persistEvent(
    client: any,
    streamId: number,
    version: number,
    event: Event
  ): Promise<void> {
    await client.query(
      `INSERT INTO events (stream_id, stream_version, event_type, event_data)
       VALUES ($1, $2, $3, $4)`,
      [streamId, version, event.type, JSON.stringify(event)]
    );
  }

  private async getEvents(client: any, streamId: number): Promise<EventEnvelope[]> {
    const result = await client.query(
      `SELECT stream_version, event_type, event_data
       FROM events
       WHERE stream_id = $1
       ORDER BY stream_version ASC`,
      [streamId]
    );
    return result.rows.map((row: any) => ({
      event: JSON.parse(row.event_data),
      metadata: { version: row.stream_version, type: row.event_type },
    }));
  }
}