import type { IOrderRepository } from '../ports/IOrderRepository.js';
import type { IUpdateOrderStatusUseCase } from '../ports/IUpdateOrderStatusUseCase.js';
import { OrderStatus } from '../domain/OrderStatus.js';

export class UpdateOrderStatusUseCase implements IUpdateOrderStatusUseCase {
  constructor(private readonly orders: IOrderRepository) {}

  async execute(orderId: string, next: OrderStatus): Promise<void> {
    await this.orders.updateStatus(orderId, next);
  }
}
