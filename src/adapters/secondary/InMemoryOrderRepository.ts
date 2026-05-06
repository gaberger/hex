import type { IOrderRepository } from '../../core/ports/IOrderRepository.js';
import type { Order, OrderId, CustomerId } from '../../core/domain/Order.js';
import { transitionOrderStatus } from '../../core/domain/Order.js';
import type { OrderStatus } from '../../core/domain/OrderStatus.js';

export class InMemoryOrderRepository implements IOrderRepository {
  private readonly store: Map<string, Order> = new Map<string, Order>();

  async findById(orderId: OrderId): Promise<Order | null> {
    return this.store.get(orderId) ?? null;
  }

  async save(order: Order): Promise<void> {
    this.store.set(order.orderId, order);
  }

  async findByCustomerId(customerId: CustomerId): Promise<Order[]> {
    const result: Order[] = [];
    for (const order of this.store.values()) {
      if (order.customerId === customerId) {
        result.push(order);
      }
    }
    return result;
  }

  async updateStatus(orderId: OrderId, status: OrderStatus): Promise<void> {
    const existing = this.store.get(orderId);
    if (!existing) {
      throw new Error(`Order not found: ${orderId}`);
    }
    const updated = transitionOrderStatus(existing, status);
    this.store.set(orderId, updated);
  }
}
