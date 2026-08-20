import { describe, it, expect } from 'vitest';
import { createOrder, transitionStatus, isCancellable } from './Order.js';
import { OrderStatus } from './OrderStatus.js';
function pending() {
  return createOrder({ orderId: 'o', customerId: 'c', restaurantId: 'r',
    items: [{ itemId: 'i', name: 'x', quantity: 1, price: { amount: 5, currency: 'USD' } }] });
}
describe('isCancellable', () => {
  it('true for Pending', () => expect(isCancellable(pending())).toBe(true));
  it('false for OutForDelivery', () => {
    let o = transitionStatus(pending(), OrderStatus.Confirmed);
    o = transitionStatus(o, OrderStatus.Preparing);
    o = transitionStatus(o, OrderStatus.OutForDelivery);
    expect(isCancellable(o)).toBe(false);
  });
});
