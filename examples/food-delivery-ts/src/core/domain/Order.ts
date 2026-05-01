import { OrderStatus } from './OrderStatus.js';

export type OrderId = string;
export type CustomerId = string;
export type RestaurantId = string;

export interface Money {
  amount: number;
  currency: string;
}

export interface OrderItem {
  itemId: string;
  name: string;
  quantity: number;
  price: Money;
}

export interface Order {
  orderId: OrderId;
  customerId: CustomerId;
  restaurantId: RestaurantId;
  items: OrderItem[];
  status: OrderStatus;
  totalAmount: Money;
  createdAt: Date;
  updatedAt: Date;
}

export interface CreateOrderParams {
  orderId: OrderId;
  customerId: CustomerId;
  restaurantId: RestaurantId;
  items: OrderItem[];
}

const VALID_STATUS_TRANSITIONS: Record<OrderStatus, OrderStatus[]> = {
  [OrderStatus.Pending]: [OrderStatus.Confirmed, OrderStatus.Cancelled],
  [OrderStatus.Confirmed]: [OrderStatus.Preparing, OrderStatus.Cancelled],
  [OrderStatus.Preparing]: [OrderStatus.OutForDelivery, OrderStatus.Cancelled],
  [OrderStatus.OutForDelivery]: [OrderStatus.Delivered, OrderStatus.Cancelled],
  [OrderStatus.Delivered]: [],
  [OrderStatus.Cancelled]: []
};

export function createOrder(params: CreateOrderParams): Order {
  if (!params.orderId || params.orderId.trim() === '') {
    throw new Error('OrderId is required');
  }
  if (!params.customerId || params.customerId.trim() === '') {
    throw new Error('CustomerId is required');
  }
  if (!params.restaurantId || params.restaurantId.trim() === '') {
    throw new Error('RestaurantId is required');
  }
  if (!params.items || params.items.length === 0) {
    throw new Error('Order must have at least one item');
  }

  const totalAmount = calculateTotalAmount(params.items);
  const now = new Date();

  return {
    orderId: params.orderId,
    customerId: params.customerId,
    restaurantId: params.restaurantId,
    items: params.items,
    status: OrderStatus.Pending,
    totalAmount,
    createdAt: now,
    updatedAt: now
  };
}

export function calculateTotalAmount(items: OrderItem[]): Money {
  if (items.length === 0) {
    return { amount: 0, currency: 'USD' };
  }

  const currency = items[0].price.currency;
  const total = items.reduce((sum, item) => {
    if (item.price.currency !== currency) {
      throw new Error('All items must have the same currency');
    }
    return sum + (item.price.amount * item.quantity);
  }, 0);

  return { amount: total, currency };
}

export function canTransitionTo(currentStatus: OrderStatus, newStatus: OrderStatus): boolean {
  const allowedTransitions = VALID_STATUS_TRANSITIONS[currentStatus];
  return allowedTransitions.includes(newStatus);
}

export function transitionStatus(order: Order, newStatus: OrderStatus): Order {
  if (!canTransitionTo(order.status, newStatus)) {
    throw new Error(
      `Invalid status transition from ${order.status} to ${newStatus}`
    );
  }

  return {
    ...order,
    status: newStatus,
    updatedAt: new Date()
  };
}
