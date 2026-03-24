// src/core/ports/P0.2.ts

import type { 
  MarketData, 
  Portfolio, 
  Trade, 
  UserCredentials 
} from '../domain/index.js';

export interface MarketDataPort {
  fetchMarketData(symbol: string): Promise<MarketData>;
  subscribeToMarketData(symbol: string, callback: (data: MarketData) => void): void;
  unsubscribeFromMarketData(symbol: string): void;
}

export interface PortfolioPersistencePort {
  savePortfolio(portfolio: Portfolio): Promise<void>;
  loadPortfolio(userId: string): Promise<Portfolio | null>;
}

export interface CachePort {
  get<T>(key: string): Promise<T | null>;
  set<T>(key: string, value: T): Promise<void>;
  invalidate(key: string): Promise<void>;
}

export interface AuditLogPort {
  logTrade(trade: Trade): Promise<void>;
  logError(error: Error): Promise<void>;
}

export interface AuthPort {
  authenticate(credentials: UserCredentials): Promise<string>;
  validateToken(token: string): Promise<boolean>;
}