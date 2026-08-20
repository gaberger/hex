import { Order, OrderId, CustomerId } from '../domain/Order.js';
import { OrderStatus } from '../domain/OrderStatus.js';

export interface IOrderRepository {
  findById(orderId: OrderId): Promise<Order | null>;
  save(order: Order): Promise<void>;
  findByCustomerId(customerId: CustomerId): Promise<Order[]>;
  updateStatus(orderId: OrderId, status: OrderStatus): Promise<void>;
}
