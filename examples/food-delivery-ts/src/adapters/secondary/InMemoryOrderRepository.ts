import { Order, OrderId, CustomerId } from '../../core/domain/Order.js';
import { OrderStatus } from '../../core/domain/OrderStatus.js';
import { IOrderRepository } from '../../core/ports/IOrderRepository.js';

export class InMemoryOrderRepository implements IOrderRepository {
  private orders: Map<string, Order>;

  constructor() {
    this.orders = new Map<string, Order>();
  }

  async findById(orderId: OrderId): Promise<Order | null> {
    return this.orders.get(orderId) ?? null;
  }

  async save(order: Order): Promise<void> {
    this.orders.set(order.id, order);
  }

  async findByCustomerId(customerId: CustomerId): Promise<Order[]> {
    return Array.from(this.orders.values()).filter(
      order => order.customerId === customerId
    );
  }

  async updateStatus(orderId: OrderId, status: OrderStatus): Promise<void> {
    const order = this.orders.get(orderId);
    if (!order) {
      throw new Error(`Order with id ${orderId} not found`);
    }

    const updatedOrder: Order = {
      ...order,
      status,
      updatedAt: new Date(),
    };

    this.orders.set(orderId, updatedOrder);
  }
}
