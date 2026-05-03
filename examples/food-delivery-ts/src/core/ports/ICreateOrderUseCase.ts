import type { Order, OrderItem } from '../domain/Order.js';

export interface CreateOrderInput {
  orderId: string;
  customerId: string;
  restaurantId: string;
  items: OrderItem[];
}

export interface ICreateOrderUseCase {
  execute(input: CreateOrderInput): Promise<Order>;
}
