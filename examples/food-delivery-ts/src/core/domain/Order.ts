import { OrderStatus } from './OrderStatus.js';

export interface OrderId {
  readonly value: string;
}

export interface CustomerId {
  readonly value: string;
}

export interface RestaurantId {
  readonly value: string;
}

export interface Money {
  readonly amount: number;
  readonly currency: string;
}

export interface OrderItem {
  readonly itemId: string;
  readonly name: string;
  readonly price: Money;
  readonly quantity: number;
}

export interface Order {
  readonly orderId: OrderId;
  readonly customerId: CustomerId;
  readonly restaurantId: RestaurantId;
  readonly items: readonly OrderItem[];
  readonly status: OrderStatus;
  readonly totalAmount: Money;
  readonly createdAt: Date;
  readonly updatedAt: Date;
}

export class InvalidStatusTransitionError extends Error {
  constructor(from: OrderStatus, to: OrderStatus) {
    super(`Invalid status transition from ${from} to ${to}`);
    this.name = 'InvalidStatusTransitionError';
  }
}

const validTransitions: Record<OrderStatus, OrderStatus[]> = {
  [OrderStatus.Pending]: [OrderStatus.Confirmed, OrderStatus.Cancelled],
  [OrderStatus.Confirmed]: [OrderStatus.Preparing, OrderStatus.Cancelled],
  [OrderStatus.Preparing]: [OrderStatus.OutForDelivery, OrderStatus.Cancelled],
  [OrderStatus.OutForDelivery]: [OrderStatus.Delivered],
  [OrderStatus.Delivered]: [],
  [OrderStatus.Cancelled]: [],
};

export function canTransitionTo(from: OrderStatus, to: OrderStatus): boolean {
  return validTransitions[from].includes(to);
}

export function transitionStatus(order: Order, newStatus: OrderStatus): Order {
  if (!canTransitionTo(order.status, newStatus)) {
    throw new InvalidStatusTransitionError(order.status, newStatus);
  }

  return {
    ...order,
    status: newStatus,
    updatedAt: new Date(),
  };
}

function calculateTotal(items: readonly OrderItem[]): Money {
  if (items.length === 0) {
    return { amount: 0, currency: 'USD' };
  }

  const currency = items[0].price.currency;
  const totalAmount = items.reduce((sum, item) => {
    if (item.price.currency !== currency) {
      throw new Error('All items must have the same currency');
    }
    return sum + item.price.amount * item.quantity;
  }, 0);

  return { amount: totalAmount, currency };
}

export function createOrder(
  orderId: OrderId,
  customerId: CustomerId,
  restaurantId: RestaurantId,
  items: readonly OrderItem[]
): Order {
  if (items.length === 0) {
    throw new Error('Order must have at least one item');
  }

  const now = new Date();
  const totalAmount = calculateTotal(items);

  return {
    orderId,
    customerId,
    restaurantId,
    items,
    status: OrderStatus.Pending,
    totalAmount,
    createdAt: now,
    updatedAt: now,
  };
}
