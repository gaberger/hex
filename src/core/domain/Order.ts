import { OrderStatus } from './OrderStatus.js';
import { Money, createMoney, addMoney } from './Money.js';

// Branded types for type safety
export type OrderId = string & { readonly __brand: 'OrderId' };
export type CustomerId = string & { readonly __brand: 'CustomerId' };
export type RestaurantId = string & { readonly __brand: 'RestaurantId' };
export type ItemId = string & { readonly __brand: 'ItemId' };

// Type constructors with validation
export function createOrderId(value: string): OrderId {
  if (!value || value.trim().length === 0) {
    throw new Error('OrderId cannot be empty');
  }
  return value as OrderId;
}

export function createCustomerId(value: string): CustomerId {
  if (!value || value.trim().length === 0) {
    throw new Error('CustomerId cannot be empty');
  }
  return value as CustomerId;
}

export function createRestaurantId(value: string): RestaurantId {
  if (!value || value.trim().length === 0) {
    throw new Error('RestaurantId cannot be empty');
  }
  return value as RestaurantId;
}

export function createItemId(value: string): ItemId {
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
  readonly orderId: OrderId;
  readonly customerId: CustomerId;
  readonly restaurantId: RestaurantId;
  readonly items: readonly OrderItem[];
  readonly status: OrderStatus;
  readonly totalAmount: Money;
  readonly createdAt: Date;
  readonly updatedAt: Date;
}

export interface CreateOrderParams {
  readonly orderId: string;
  readonly customerId: string;
  readonly restaurantId: string;
  readonly items: readonly OrderItem[];
}

/**
 * Factory method to create a new Order
 */
export function createOrder(params: CreateOrderParams): Order {
  const { orderId, customerId, restaurantId, items } = params;

  // Validate IDs
  if (!orderId || orderId.trim().length === 0) {
    throw new Error('Order ID is required');
  }
  if (!customerId || customerId.trim().length === 0) {
    throw new Error('Customer ID is required');
  }
  if (!restaurantId || restaurantId.trim().length === 0) {
    throw new Error('Restaurant ID is required');
  }

  if (!items || items.length === 0) {
    throw new Error('Order must contain at least one item');
  }

  // Validate all items have positive quantities
  for (const item of items) {
    if (item.quantity <= 0) {
      throw new Error(`Item ${item.itemId} must have a positive quantity`);
    }
  }

  // Ensure all items have the same currency
  const currency = items[0].unitPrice.currency;
  if (!items.every(item => item.unitPrice.currency === currency)) {
    throw new Error('All items must have the same currency');
  }

  // Calculate total amount
  const totalAmount = items.reduce((total, item) => {
    const itemTotal = createMoney(
      item.unitPrice.amount * item.quantity,
      item.unitPrice.currency
    );
    return addMoney(total, itemTotal);
  }, createMoney(0, currency));

  const now = new Date();

  return {
    orderId: createOrderId(orderId),
    customerId: createCustomerId(customerId),
    restaurantId: createRestaurantId(restaurantId),
    items: items,
    status: OrderStatus.Pending,
    totalAmount,
    createdAt: now,
    updatedAt: now,
  };
}

/**
 * Valid status transitions according to business rules
 */
const VALID_TRANSITIONS: Record<OrderStatus, OrderStatus[]> = {
  [OrderStatus.Pending]: [OrderStatus.Confirmed, OrderStatus.Cancelled],
  [OrderStatus.Confirmed]: [OrderStatus.Preparing, OrderStatus.Cancelled],
  [OrderStatus.Preparing]: [OrderStatus.OutForDelivery, OrderStatus.Cancelled],
  [OrderStatus.OutForDelivery]: [OrderStatus.Delivered, OrderStatus.Cancelled],
  [OrderStatus.Delivered]: [],
  [OrderStatus.Cancelled]: [],
};

/**
 * Validates if a status transition is allowed
 */
export function isValidStatusTransition(
  currentStatus: OrderStatus,
  newStatus: OrderStatus
): boolean {
  const allowedTransitions = VALID_TRANSITIONS[currentStatus];
  return allowedTransitions.includes(newStatus);
}

/**
 * Transitions order to a new status with validation
 */
export function transitionOrderStatus(
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

/**
 * Updates an existing order with new data
 */
export function updateOrder(
  order: Order,
  updates: Partial<Pick<Order, 'items' | 'status'>>
): Order {
  const updatedOrder = { ...order, ...updates, updatedAt: new Date() };

  // Recalculate total if items changed
  if (updates.items && updates.items.length > 0) {
    const totalAmount = updates.items.reduce((total, item) => {
      const itemTotal = createMoney(
        item.unitPrice.amount * item.quantity,
        item.unitPrice.currency
      );
      return addMoney(total, itemTotal);
    }, createMoney(0, updates.items[0].unitPrice.currency));

    return { ...updatedOrder, totalAmount };
  }

  return updatedOrder;
}
