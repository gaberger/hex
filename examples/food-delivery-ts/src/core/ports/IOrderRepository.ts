import { Order } from '../domain/Order.js';
import { OrderStatus } from '../domain/OrderStatus.js';

export interface IOrderRepository {
  findById(id: string): Promise<Order | null>;
  save(order: Order): Promise<void>;
  findByStatus(status: OrderStatus): Promise<Order[]>;
  delete(id: string): Promise<void>;
}
