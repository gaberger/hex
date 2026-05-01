import { Order, OrderId, CustomerId, transitionStatus } from '../../core/domain/Order.js';
import { OrderStatus } from '../../core/domain/OrderStatus.js';
import { IOrderRepository } from '../../core/ports/IOrderRepository.js';

export class InMemoryOrderRepository implements IOrderRepository {
  private storage: Map<string, Order>;

  constructor() {
    this.storage = new Map<string, Order>();
  }

  async save(order: Order): Promise<void> {
    this.storage.set(order.orderId.value, order);
  }

  async findById(orderId: OrderId): Promise<Order | null> {
    return this.storage.get(orderId.value) || null;
  }

  async findByCustomerId(customerId: CustomerId): Promise<Order[]> {
    return Array.from(this.storage.values()).filter(
      order => order.customerId.value === customerId.value
    );
  }

  async updateStatus(orderId: OrderId, status: OrderStatus): Promise<void> {
    const order = await this.findById(orderId);
    if (!order) {
      throw new Error(`Order with id ${orderId.value} not found`);
    }

    const updatedOrder = transitionStatus(order, status);
    await this.save(updatedOrder);
  }
}