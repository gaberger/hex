import type { OrderStatus } from '../domain/OrderStatus.js';

export interface IUpdateOrderStatusUseCase {
  execute(orderId: string, next: OrderStatus): Promise<void>;
}
