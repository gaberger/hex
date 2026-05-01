import { Money } from './Money.js';
import { OrderStatus, isValidStatusTransition } from './OrderStatus.js';

export type OrderId = string & { readonly __brand: 'OrderId' };
export type CustomerId = string & { readonly __brand: 'CustomerId' };
export type RestaurantId = string & { readonly __brand: 'RestaurantId' };
export type ItemId = string & { readonly __brand: 'ItemId' };

export function OrderId(value: string): OrderId {
  if (!value || value.trim().length === 0) {
    throw new Error('OrderId cannot be empty');
  }
  return value as OrderId;
}

export function CustomerId(value: string): CustomerId {
  if (!value || value.trim().length === 0) {
    throw new Error('CustomerId cannot be empty');
  }
  return value as CustomerId;
}

export function RestaurantId(value: string): RestaurantId {
  if (!value || value.trim().length === 0) {
    throw new Error('RestaurantId cannot be empty');
  }
  return value as RestaurantId;
}

export function ItemId(value: string): ItemId {
  if (!value || value.trim().length === 0) {
    throw new Error('ItemId cannot be empty');
  }
  return value as ItemId;
}

export interface OrderItem {
  readonly itemId: ItemId;
  readonly name: string;
  readonly quantity: number;
  readonly unitPrice: Money;
}

export interface Order {
  readonly id: OrderId;
  readonly customerId: CustomerId;
  readonly restaurantId: RestaurantId;
  readonly items: readonly OrderItem[];
  readonly status: OrderStatus;
  readonly totalAmount: Money;
  readonly createdAt: Date;
  readonly updatedAt: Date;
}

export interface CreateOrderParams {
  readonly customerId: CustomerId;
  readonly restaurantId: RestaurantId;
  readonly items: readonly OrderItem[];
}

export function createOrder(params: CreateOrderParams): Order {
  const { customerId, restaurantId, items } = params;

  if (items.length === 0) {
    throw new Error('Order must contain at least one item');
  }

  const currency = items[0].unitPrice.currency;
  if (!items.every(item => item.unitPrice.currency === currency)) {
    throw new Error('All items must have the same currency');
  }

  if (!items.every(item => item.quantity > 0)) {
    throw new Error('All items must have positive quantity');
  }

  const totalAmount = items.reduce(
    (sum, item) => sum.add(
      Money.of(item.unitPrice.amount * item.quantity, item.unitPrice.currency)
    ),
    Money.of(0, currency)
  );

  const now = new Date();

  return {
    id: OrderId(crypto.randomUUID()),
    customerId,
    restaurantId,
    items,
    status: OrderStatus.Pending,
    totalAmount,
    createdAt: now,
    updatedAt: now,
  };
}

export function transitionStatus(
  order: Order,
  newStatus: OrderStatus
): Order {
  if (!isValidStatusTransition(order.status, newStatus)) {
    throw new Error(
      `Invalid status transition from ${order.status} to ${newStatus}`
    );
  }

  return {
    ...order,
    status: newStatus,
    updatedAt: new Date(),
  };
}

export function calculateTotal(items: readonly OrderItem[]): Money {
  if (items.length === 0) {
    throw new Error('Cannot calculate total for empty items list');
  }

  const currency = items[0].unitPrice.currency;
  return items.reduce(
    (sum, item) => sum.add(
      Money.of(item.unitPrice.amount * item.quantity, item.unitPrice.currency)
    ),
    Money.of(0, currency)
  );
}
