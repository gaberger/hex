/**
 * types/ — Frontend port types and domain interfaces (ADR-056).
 */
export type { ChatMessage } from './chat';
export type { GitStatus, StatusFile, WorktreeInfo, CommitInfo, LogResult, BranchInfo, DiffFile, DiffResult } from './git';
export type { Project, InitResult } from './project';
export type { IRestClient, IWebSocketTransport, IChatTransport, IStorageAdapter, MessageHandler, StatusHandler } from './services';
