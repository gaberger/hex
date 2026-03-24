import { CacheAdapter } from '../../ports/CacheAdapter.js';
import { createClient, RedisClientType } from 'redis';

export class RedisCacheAdapter implements CacheAdapter {
  private readonly client: RedisClientType;

  constructor(url: string) {
    this.client = createClient({ url });
    this.client.on('error', (err) => console.error('Redis Client Error', err));
  }

  async init(): Promise<void> {
    await this.client.connect();
  }

  async get<T>(key: string): Promise<T | null> {
    const value = await this.client.get(key);
    return value ? JSON.parse(value) as T : null;
  }

  async set<T>(key: string, value: T, ttlSeconds?: number): Promise<void> {
    const stringifiedValue = JSON.stringify(value);
    if (ttlSeconds) {
      await this.client.set(key, stringifiedValue, { EX: ttlSeconds });
    } else {
      await this.client.set(key, stringifiedValue);
    }
  }

  async delete(key: string): Promise<void> {
    await this.client.del(key);
  }

  async exists(key: string): Promise<boolean> {
    return (await this.client.exists(key)) === 1;
  }
}