import { describe, it, expect } from 'vitest';
import { createOrder, transitionStatus, cancelOrder, InvalidStatusTransitionError } from './Order.js';
import { OrderStatus } from './OrderStatus.js';

// Behavioral spec (oracle) for the order-cancellation feature. The do-loop implements
// `cancelOrder` in Order.ts to satisfy this; it does not edit this file.

function pendingOrder() {
  return createOrder({
    orderId: 'o1',
    customerId: 'c1',
    restaurantId: 'r1',
    items: [{ itemId: 'i1', name: 'Pizza', quantity: 1, price: { amount: 10, currency: 'USD' } }],
  });
}

describe('cancelOrder', () => {
  it('cancels a Pending order', () => {
    expect(cancelOrder(pendingOrder()).status).toBe(OrderStatus.Cancelled);
  });

  it('cancels a Confirmed order', () => {
    const confirmed = transitionStatus(pendingOrder(), OrderStatus.Confirmed);
    expect(cancelOrder(confirmed).status).toBe(OrderStatus.Cancelled);
  });

  it('cancels a Preparing order', () => {
    let o = transitionStatus(pendingOrder(), OrderStatus.Confirmed);
    o = transitionStatus(o, OrderStatus.Preparing);
    expect(cancelOrder(o).status).toBe(OrderStatus.Cancelled);
  });

  it('refuses to cancel a Delivered order', () => {
    let o = transitionStatus(pendingOrder(), OrderStatus.Confirmed);
    o = transitionStatus(o, OrderStatus.Preparing);
    o = transitionStatus(o, OrderStatus.OutForDelivery);
    o = transitionStatus(o, OrderStatus.Delivered);
    expect(() => cancelOrder(o)).toThrow(InvalidStatusTransitionError);
  });

  it('refuses to cancel an OutForDelivery order (not an allowed transition)', () => {
    let o = transitionStatus(pendingOrder(), OrderStatus.Confirmed);
    o = transitionStatus(o, OrderStatus.Preparing);
    o = transitionStatus(o, OrderStatus.OutForDelivery);
    expect(() => cancelOrder(o)).toThrow(InvalidStatusTransitionError);
  });

  it('does not mutate the original order', () => {
    const original = pendingOrder();
    cancelOrder(original);
    expect(original.status).toBe(OrderStatus.Pending);
  });
});
