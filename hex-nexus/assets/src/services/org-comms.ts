/**
 * org-comms.ts — Organization communication API client
 */

import { restClient } from "./rest-client";

export interface SendMessageRequest {
  from: string; // "ceo" for user
  content: string;
  context?: any;
  project_id?: string;
}

export interface SendMessageResponse {
  message_id: string;
  routed_to: string[];
  status: string;
  project_scope?: string;
}

export interface ConversationMessage {
  id: string;
  from: string;
  to: string;
  content: string;
  timestamp: string;
  status: "sent" | "routing" | "delegated" | "processing" | "completed";
}

export interface ConversationThread {
  id: string;
  messages: ConversationMessage[];
  active_agents: string[];
}

export const orgCommsClient = {
  async sendMessage(req: SendMessageRequest): Promise<SendMessageResponse> {
    return restClient.post("/api/org/send-message", req);
  },

  async getConversation(id: string): Promise<ConversationThread> {
    return restClient.get(`/api/org/conversation/${id}`);
  },
};
