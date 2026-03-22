/**
 * services/ — Secondary adapters barrel (ADR-056).
 *
 * All I/O services are singletons exported from here.
 * Stores import services; components import stores. Never the reverse.
 */
export { restClient } from './rest-client';
export { chatWs } from './chat-ws';
export { gitWs } from './git-ws';
export { storage } from './local-storage';
export { createProjectChatTransport } from './project-chat-ws';
