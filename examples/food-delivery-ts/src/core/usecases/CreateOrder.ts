import type { IOrderRepository } from '../ports/IOrderRepository.js';
import type { ICreateOrderUseCase, CreateOrderInput } from '../ports/ICreateOrderUseCase.js';
import { createOrder, type Order } from '../domain/Order.js';

export type { CreateOrderInput };

export class CreateOrderUseCase implements ICreateOrderUseCase {
  constructor(private readonly orders: IOrderRepository) {}

  async execute(input: CreateOrderInput): Promise<Order> {
    const order = createOrder({
      orderId: input.orderId,
      customerId: input.customerId,
      restaurantId: input.restaurantId,
      items: input.items,
    });
    await this.orders.save(order);
    return order;
  }
}
