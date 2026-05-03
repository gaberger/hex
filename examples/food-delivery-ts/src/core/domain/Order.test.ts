import { describe, it, expect, beforeEach } from 'vitest';
import {
  createOrder,
  transitionStatus,
  canTransitionTo,
  InvalidStatusTransitionError,
  type Order,
  type OrderId,
  type CustomerId,
  type RestaurantId,
  type OrderItem,
} from './Order.js';
import { OrderStatus } from './OrderStatus.js';

describe('Order Domain', () => {
  const mockOrderId: OrderId = 'order-123';
  const mockCustomerId: CustomerId = 'customer-456';
  const mockRestaurantId: RestaurantId = 'restaurant-789';

  const createMockItem = (
    id: string,
    name: string,
    amount: number,
    quantity: number,
    currency = 'USD'
  ): OrderItem => ({
    itemId: id,
    name,
    price: { amount, currency },
    quantity,
  });

  const makeOrder = (items: OrderItem[]): Order =>
    createOrder({
      orderId: mockOrderId,
      customerId: mockCustomerId,
      restaurantId: mockRestaurantId,
      items,
    });

  describe('createOrder', () => {
    it('should create a valid order with correct properties', () => {
      const items: OrderItem[] = [
        createMockItem('item-1', 'Burger', 10.5, 2),
        createMockItem('item-2', 'Fries', 3.5, 1),
      ];
      const order = makeOrder(items);
      expect(order.orderId).toBe(mockOrderId);
      expect(order.customerId).toBe(mockCustomerId);
      expect(order.restaurantId).toBe(mockRestaurantId);
      expect(order.items).toEqual(items);
      expect(order.status).toBe(OrderStatus.Pending);
      expect(order.createdAt).toBeInstanceOf(Date);
      expect(order.updatedAt).toBeInstanceOf(Date);
      expect(order.createdAt.getTime()).toBe(order.updatedAt.getTime());
    });

    it('single item totalAmount', () => {
      const order = makeOrder([createMockItem('i', 'Pizza', 15.99, 1)]);
      expect(order.totalAmount).toEqual({ amount: 15.99, currency: 'USD' });
    });

    it('multi-item totalAmount', () => {
      const order = makeOrder([
        createMockItem('a', 'Burger', 10.5, 2),
        createMockItem('b', 'Fries', 3.5, 1),
        createMockItem('c', 'Soda', 2.0, 3),
      ]);
      expect(order.totalAmount).toEqual({ amount: 30.5, currency: 'USD' });
    });

    it('quantity multiplier', () => {
      const order = makeOrder([createMockItem('a', 'Taco', 4.25, 5)]);
      expect(order.totalAmount).toEqual({ amount: 21.25, currency: 'USD' });
    });

    it('rejects empty items', () => {
      expect(() => makeOrder([])).toThrow('Order must have at least one item');
    });

    it('rejects mixed currencies', () => {
      expect(() =>
        makeOrder([
          createMockItem('a', 'Burger', 10.5, 1, 'USD'),
          createMockItem('b', 'Fries', 3.5, 1, 'EUR'),
        ])
      ).toThrow('All items must have the same currency');
    });

    it('non-USD currency', () => {
      const order = makeOrder([
        createMockItem('a', 'Pasta', 12.0, 2, 'EUR'),
        createMockItem('b', 'Wine', 8.0, 1, 'EUR'),
      ]);
      expect(order.totalAmount).toEqual({ amount: 32.0, currency: 'EUR' });
    });

    it('zero-amount items', () => {
      const order = makeOrder([
        createMockItem('a', 'Free', 0, 1),
        createMockItem('b', 'Burger', 10.5, 1),
      ]);
      expect(order.totalAmount).toEqual({ amount: 10.5, currency: 'USD' });
    });
  });

  describe('Status Transitions', () => {
    let baseOrder: Order;
    beforeEach(() => {
      baseOrder = makeOrder([createMockItem('a', 'Burger', 10.5, 1)]);
    });

    describe('canTransitionTo', () => {
      it('Pending → Confirmed', () => {
        expect(canTransitionTo(OrderStatus.Pending, OrderStatus.Confirmed)).toBe(true);
      });
      it('Pending → Cancelled', () => {
        expect(canTransitionTo(OrderStatus.Pending, OrderStatus.Cancelled)).toBe(true);
      });
      it('Confirmed → Preparing', () => {
        expect(canTransitionTo(OrderStatus.Confirmed, OrderStatus.Preparing)).toBe(true);
      });
      it('Confirmed → Cancelled', () => {
        expect(canTransitionTo(OrderStatus.Confirmed, OrderStatus.Cancelled)).toBe(true);
      });
      it('Preparing → OutForDelivery', () => {
        expect(canTransitionTo(OrderStatus.Preparing, OrderStatus.OutForDelivery)).toBe(true);
      });
      it('Preparing → Cancelled', () => {
        expect(canTransitionTo(OrderStatus.Preparing, OrderStatus.Cancelled)).toBe(true);
      });
      it('OutForDelivery → Delivered', () => {
        expect(canTransitionTo(OrderStatus.OutForDelivery, OrderStatus.Delivered)).toBe(true);
      });
      it('blocks Pending → Preparing', () => {
        expect(canTransitionTo(OrderStatus.Pending, OrderStatus.Preparing)).toBe(false);
      });
      it('blocks Pending → Delivered', () => {
        expect(canTransitionTo(OrderStatus.Pending, OrderStatus.Delivered)).toBe(false);
      });
      it('blocks Delivered → anything', () => {
        expect(canTransitionTo(OrderStatus.Delivered, OrderStatus.Pending)).toBe(false);
        expect(canTransitionTo(OrderStatus.Delivered, OrderStatus.Cancelled)).toBe(false);
      });
      it('blocks Cancelled → anything', () => {
        expect(canTransitionTo(OrderStatus.Cancelled, OrderStatus.Pending)).toBe(false);
        expect(canTransitionTo(OrderStatus.Cancelled, OrderStatus.Confirmed)).toBe(false);
      });
      it('blocks OutForDelivery → Cancelled', () => {
        expect(canTransitionTo(OrderStatus.OutForDelivery, OrderStatus.Cancelled)).toBe(false);
      });
    });

    describe('transitionStatus', () => {
      it('Pending → Confirmed', () => {
        const updated = transitionStatus(baseOrder, OrderStatus.Confirmed);
        expect(updated.status).toBe(OrderStatus.Confirmed);
        expect(updated.updatedAt.getTime()).toBeGreaterThanOrEqual(baseOrder.updatedAt.getTime());
      });
      it('happy path: Pending → Confirmed → Preparing', () => {
        const confirmed = transitionStatus(baseOrder, OrderStatus.Confirmed);
        const preparing = transitionStatus(confirmed, OrderStatus.Preparing);
        expect(preparing.status).toBe(OrderStatus.Preparing);
      });
      it('full delivery flow', () => {
        let order = baseOrder;
        order = transitionStatus(order, OrderStatus.Confirmed);
        order = transitionStatus(order, OrderStatus.Preparing);
        order = transitionStatus(order, OrderStatus.OutForDelivery);
        order = transitionStatus(order, OrderStatus.Delivered);
        expect(order.status).toBe(OrderStatus.Delivered);
      });
      it('cancel from Pending', () => {
        expect(transitionStatus(baseOrder, OrderStatus.Cancelled).status).toBe(OrderStatus.Cancelled);
      });
      it('cancel from Confirmed', () => {
        const c = transitionStatus(baseOrder, OrderStatus.Confirmed);
        expect(transitionStatus(c, OrderStatus.Cancelled).status).toBe(OrderStatus.Cancelled);
      });
      it('cancel from Preparing', () => {
        let o = transitionStatus(baseOrder, OrderStatus.Confirmed);
        o = transitionStatus(o, OrderStatus.Preparing);
        expect(transitionStatus(o, OrderStatus.Cancelled).status).toBe(OrderStatus.Cancelled);
      });
      it('updates updatedAt', () => {
        const before = new Date();
        const u = transitionStatus(baseOrder, OrderStatus.Confirmed);
        expect(u.updatedAt.getTime()).toBeGreaterThanOrEqual(before.getTime());
      });
      it('preserves other props', () => {
        const u = transitionStatus(baseOrder, OrderStatus.Confirmed);
        expect(u.orderId).toBe(baseOrder.orderId);
        expect(u.customerId).toBe(baseOrder.customerId);
        expect(u.restaurantId).toBe(baseOrder.restaurantId);
        expect(u.items).toEqual(baseOrder.items);
        expect(u.totalAmount).toEqual(baseOrder.totalAmount);
        expect(u.createdAt).toEqual(baseOrder.createdAt);
      });
      it('throws InvalidStatusTransitionError for invalid', () => {
        expect(() => transitionStatus(baseOrder, OrderStatus.Preparing)).toThrow(
          InvalidStatusTransitionError
        );
      });
      it('throws from Delivered', () => {
        let o = baseOrder;
        o = transitionStatus(o, OrderStatus.Confirmed);
        o = transitionStatus(o, OrderStatus.Preparing);
        o = transitionStatus(o, OrderStatus.OutForDelivery);
        o = transitionStatus(o, OrderStatus.Delivered);
        expect(() => transitionStatus(o, OrderStatus.Pending)).toThrow(InvalidStatusTransitionError);
      });
      it('throws from Cancelled', () => {
        const c = transitionStatus(baseOrder, OrderStatus.Cancelled);
        expect(() => transitionStatus(c, OrderStatus.Confirmed)).toThrow(InvalidStatusTransitionError);
      });
      it('rejects skipping statuses', () => {
        expect(() => transitionStatus(baseOrder, OrderStatus.OutForDelivery)).toThrow(
          InvalidStatusTransitionError
        );
      });
    });
  });

  describe('Validation Rules', () => {
    it('min item quantity 1', () => {
      const o = makeOrder([createMockItem('a', 'Burger', 10.5, 1)]);
      expect(o.items[0].quantity).toBeGreaterThan(0);
    });
    it('large quantities', () => {
      const o = makeOrder([createMockItem('a', 'Napkin', 0.05, 100)]);
      expect(o.totalAmount).toEqual({ amount: 5.0, currency: 'USD' });
    });
    it('decimal prices', () => {
      const o = makeOrder([
        createMockItem('a', 'Coffee', 3.99, 1),
        createMockItem('b', 'Muffin', 2.49, 1),
      ]);
      expect(o.totalAmount.amount).toBeCloseTo(6.48, 2);
    });
    it('immutability', () => {
      const o = makeOrder([createMockItem('a', 'Burger', 10.5, 1)]);
      const t = transitionStatus(o, OrderStatus.Confirmed);
      expect(o.status).toBe(OrderStatus.Pending);
      expect(t.status).toBe(OrderStatus.Confirmed);
      expect(o).not.toBe(t);
    });
  });

  describe('InvalidStatusTransitionError', () => {
    it('correct message', () => {
      const e = new InvalidStatusTransitionError(OrderStatus.Pending, OrderStatus.Preparing);
      expect(e.message).toBe('Invalid status transition from Pending to Preparing');
      expect(e.name).toBe('InvalidStatusTransitionError');
    });
    it('throwable + catchable', () => {
      const o = makeOrder([createMockItem('a', 'Burger', 10.5, 1)]);
      try {
        transitionStatus(o, OrderStatus.Preparing);
        expect.fail('should throw');
      } catch (e) {
        expect(e).toBeInstanceOf(InvalidStatusTransitionError);
      }
    });
  });
});
