import { WebSocket } from 'ws';
import { MarketDataPort } from '../../ports/secondary/MarketDataPort.js';
import { MarketData } from '../../domain/MarketData.js';

export class MarketDataWebSocketAdapter implements MarketDataPort {
  private ws: WebSocket;

  constructor(url: string) {
    this.ws = new WebSocket(url);
    this.ws.on('message', this.handleMessage.bind(this));
    this.ws.on('error', this.handleError.bind(this));
  }

  private handleMessage(data: string): void {
    try {
      const marketData: MarketData = JSON.parse(data);
      this.onMarketData(marketData);
    } catch (error) {
      console.error('Error parsing market data:', error);
    }
  }

  private handleError(error: Error): void {
    console.error('WebSocket error:', error);
  }

  private onMarketData(marketData: MarketData): void {
    // This method should be implemented according to the MarketDataPort interface
    // For demonstration purposes, it's assumed that MarketDataPort has a method 'onMarketData'
    // that needs to be called when market data is received.
    // In a real implementation, you should call the appropriate method on the port.
    console.log('Received market data:', marketData);
  }

  public connect(): void {
    this.ws.on('open', () => {
      console.log('Connected to WebSocket market data feed');
    });
  }

  public disconnect(): void {
    this.ws.close();
  }

  public subscribe(symbols: string[]): void {
    // Assuming the WebSocket API requires sending a subscription message
    const subscriptionMessage = JSON.stringify({ type: 'subscribe', symbols });
    this.ws.send(subscriptionMessage);
  }

  public unsubscribe(symbols: string[]): void {
    // Assuming the WebSocket API requires sending an unsubscription message
    const unsubscriptionMessage = JSON.stringify({ type: 'unsubscribe', symbols });
    this.ws.send(unsubscriptionMessage);
  }
}