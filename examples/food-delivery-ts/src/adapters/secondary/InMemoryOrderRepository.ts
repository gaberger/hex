import { IOrderRepository } from '../../core/ports/IOrderRepository.js';
import { Order, OrderId, CustomerId, transitionStatus } from '../../core/domain/Order.js';
import { OrderStatus } from '../../core/domain/OrderStatus.js';

export class InMemoryOrderRepository implements IOrderRepository {
  private orders: Map<string, Order>;

  constructor() {
    this.orders = new Map<string, Order>();
  }

  async findById(orderId: OrderId): Promise<Order | null> {
    return this.orders.get(orderId) || null;
  }

  async save(order: Order): Promise<void> {
    this.orders.set(order.orderId, order);
  }

  async findByCustomerId(customerId: CustomerId): Promise<Order[]> {
    const customerOrders: Order[] = [];
    for (const order of this.orders.values()) {
      if (order.customerId === customerId) {
        customerOrders.push(order);
      }
    }
    return customerOrders;
  }

  async updateStatus(orderId: OrderId, status: OrderStatus): Promise<void> {
    const order = this.orders.get(orderId);
    if (!order) {
      throw new Error(`Order not found: ${orderId}`);
    }
    const updatedOrder = transitionStatus(order, status);
    this.orders.set(orderId, updatedOrder);
  }
}
