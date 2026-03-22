/**
 * chat.ts — Chat domain types (ADR-056).
 * Shared between stores/chat.ts and components/chat/.
 */

export interface ChatMessage {
  id: string;
  role: 'user' | 'assistant' | 'system' | 'tool';
  content: string;
  model?: string;
  timestamp: string;
  toolName?: string;
  toolInput?: string;
  toolResult?: string;
  toolUseId?: string;
  isError?: boolean;
}
