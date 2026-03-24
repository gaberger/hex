import { WebSocket } from 'ws';
import { MarketDataPublisher } from '../../../ports/primary/MarketDataPublisher.js';
import { Logger } from '../../../ports/secondary/Logger.js';

export class MarketDataWebSocketHandler {
  private readonly wss: WebSocket.Server;
  private readonly marketDataPublisher: MarketDataPublisher;
  private readonly logger: Logger;

  constructor(wss: WebSocket.Server, marketDataPublisher: MarketDataPublisher, logger: Logger) {
    this.wss = wss;
    this.marketDataPublisher = marketDataPublisher;
    this.logger = logger;

    this.wss.on('connection', (ws) => this.handleConnection(ws));
  }

  private handleConnection(ws: WebSocket): void {
    this.logger.info('New WebSocket connection established');

    ws.on('message', (message) => this.handleMessage(ws, message));
    ws.on('close', () => this.handleClose(ws));
    ws.on('error', (error) => this.handleError(ws, error));

    this.marketDataPublisher.subscribe((data) => this.broadcast(data));
  }

  private handleMessage(ws: WebSocket, message: WebSocket.RawData): void {
    try {
      const parsedMessage = JSON.parse(message.toString());
      this.logger.info(`Received message: ${JSON.stringify(parsedMessage)}`);
      // Handle message logic here if needed
    } catch (error) {
      this.logger.error(`Error parsing message: ${error}`);
      ws.send(JSON.stringify({ error: 'Invalid message format' }));
    }
  }

  private handleClose(ws: WebSocket): void {
    this.logger.info('WebSocket connection closed');
    // Handle connection close logic here if needed
  }

  private handleError(ws: WebSocket, error: Error): void {
    this.logger.error(`WebSocket error: ${error}`);
    ws.send(JSON.stringify({ error: 'Internal server error' }));
  }

  private broadcast(data: unknown): void {
    this.wss.clients.forEach((client) => {
      if (client.readyState === WebSocket.OPEN) {
        client.send(JSON.stringify(data));
      }
    });
  }
}