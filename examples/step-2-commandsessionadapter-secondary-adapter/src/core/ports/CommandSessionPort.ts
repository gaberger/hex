import { Result } from '../domain/value-objects/Result.js';

export interface CommandSession {
  id: string;
  createdAt: Date;
  lastAccessedAt: Date;
}

export interface CommandSessionPort {
  createSession(): Promise<Result<CommandSession, Error>>;
  getSession(sessionId: string): Promise<Result<CommandSession | null, Error>>;
  updateLastAccessed(sessionId: string): Promise<Result<void, Error>>;
  deleteSession(sessionId: string): Promise<Result<void, Error>>;
  listActiveSessions(): Promise<Result<CommandSession[], Error>>;
}