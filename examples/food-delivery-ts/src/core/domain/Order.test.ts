import { describe, it, expect } from 'vitest';
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
  type Money,
} from './Order.js';
import { OrderStatus } from './OrderStatus.js';

describe('Order Domain', () => {
  const mockOrderId: OrderId = { value: 'order-123' };
  const mockCustomerId: CustomerId = { value: 'customer-456' };
  const mockRestaurantId: RestaurantId = { value: 'restaurant-789' };

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

  describe('createOrder', () => {
    it('should create a valid order with correct properties', () => {
      const items: OrderItem[] = [
        createMockItem('item-1', 'Burger', 10.5, 2),
        createMockItem('item-2', 'Fries', 3.5, 1),
      ];

      const order = createOrder(mockOrderId, mockCustomerId, mockRestaurantId, items);

      expect(order.orderId).toEqual(mockOrderId);
      expect(order.customerId).toEqual(mockCustomerId);
      expect(order.restaurantId).toEqual(mockRestaurantId);
      expect(order.items).toEqual(items);
      expect(order.status).toBe(OrderStatus.Pending);
      expect(order.createdAt).toBeInstanceOf(Date);
      expect(order.updatedAt).toBeInstanceOf(Date);
      expect(order.createdAt.getTime()).toBe(order.updatedAt.getTime());
    });

    it('should calculate totalAmount correctly for single item', () => {
      const items: OrderItem[] = [createMockItem('item-1', 'Pizza', 15.99, 1)];

      const order = createOrder(mockOrderId, mockCustomerId, mockRestaurantId, items);

      expect(order.totalAmount).toEqual({ amount: 15.99, currency: 'USD' });
    });

    it('should calculate totalAmount correctly for multiple items', () => {
      const items: OrderItem[] = [
        createMockItem('item-1', 'Burger', 10.5, 2), // 21.0
        createMockItem('item-2', 'Fries', 3.5, 1), // 3.5
        createMockItem('item-3', 'Soda', 2.0, 3), // 6.0
      ];

      const order = createOrder(mockOrderId, mockCustomerId, mockRestaurantId, items);

      expect(order.totalAmount).toEqual({ amount: 30.5, currency: 'USD' });
    });

    it('should calculate totalAmount with quantity multiplier', () => {
      const items: OrderItem[] = [createMockItem('item-1', 'Taco', 4.25, 5)];

      const order = createOrder(mockOrderId, mockCustomerId, mockRestaurantId, items);

      expect(order.totalAmount).toEqual({ amount: 21.25, currency: 'USD' });
    });

    it('should throw error when creating order with no items', () => {
      expect(() => createOrder(mockOrderId, mockCustomerId, mockRestaurantId, [])).toThrow(
        'Order must have at least one item'
      );
    });

    it('should throw error when items have different currencies', () => {
      const items: OrderItem[] = [
        createMockItem('item-1', 'Burger', 10.5, 1, 'USD'),
        createMockItem('item-2', 'Fries', 3.5, 1, 'EUR'),
      ];

      expect(() => createOrder(mockOrderId, mockCustomerId, mockRestaurantId, items)).toThrow(
        'All items must have the same currency'
      );
    });

    it('should handle non-USD currency', () => {
      const items: OrderItem[] = [
        createMockItem('item-1', 'Pasta', 12.0, 2, 'EUR'),
        createMockItem('item-2', 'Wine', 8.0, 1, 'EUR'),
      ];

      const order = createOrder(mockOrderId, mockCustomerId, mockRestaurantId, items);

      expect(order.totalAmount).toEqual({ amount: 32.0, currency: 'EUR' });
    });

    it('should handle zero-amount items', () => {
      const items: OrderItem[] = [
        createMockItem('item-1', 'Free Sample', 0, 1),
        createMockItem('item-2', 'Burger', 10.5, 1),
      ];

      const order = createOrder(mockOrderId, mockCustomerId, mockRestaurantId, items);

      expect(order.totalAmount).toEqual({ amount: 10.5, currency: 'USD' });
    });
  });

  describe('Status Transitions', () => {
    let baseOrder: Order;

    beforeEach(() => {
      const items: OrderItem[] = [createMockItem('item-1', 'Burger', 10.5, 1)];
      baseOrder = createOrder(mockOrderId, mockCustomerId, mockRestaurantId, items);
    });

    describe('canTransitionTo', () => {
      it('should allow Pending → Confirmed transition', () => {
        expect(canTransitionTo(OrderStatus.Pending, OrderStatus.Confirmed)).toBe(true);
      });

      it('should allow Pending → Cancelled transition', () => {
        expect(canTransitionTo(OrderStatus.Pending, OrderStatus.Cancelled)).toBe(true);
      });

      it('should allow Confirmed → Preparing transition', () => {
        expect(canTransitionTo(OrderStatus.Confirmed, OrderStatus.Preparing)).toBe(true);
      });

      it('should allow Confirmed → Cancelled transition', () => {
        expect(canTransitionTo(OrderStatus.Confirmed, OrderStatus.Cancelled)).toBe(true);
      });

      it('should allow Preparing → OutForDelivery transition', () => {
        expect(canTransitionTo(OrderStatus.Preparing, OrderStatus.OutForDelivery)).toBe(true);
      });

      it('should allow Preparing → Cancelled transition', () => {
        expect(canTransitionTo(OrderStatus.Preparing, OrderStatus.Cancelled)).toBe(true);
      });

      it('should allow OutForDelivery → Delivered transition', () => {
        expect(canTransitionTo(OrderStatus.OutForDelivery, OrderStatus.Delivered)).toBe(true);
      });

      it('should not allow Pending → Preparing transition', () => {
        expect(canTransitionTo(OrderStatus.Pending, OrderStatus.Preparing)).toBe(false);
      });

      it('should not allow Pending → Delivered transition', () => {
        expect(canTransitionTo(OrderStatus.Pending, OrderStatus.Delivered)).toBe(false);
      });

      it('should not allow Delivered → any transition', () => {
        expect(canTransitionTo(OrderStatus.Delivered, OrderStatus.Pending)).toBe(false);
        expect(canTransitionTo(OrderStatus.Delivered, OrderStatus.Cancelled)).toBe(false);
      });

      it('should not allow Cancelled → any transition', () => {
        expect(canTransitionTo(OrderStatus.Cancelled, OrderStatus.Pending)).toBe(false);
        expect(canTransitionTo(OrderStatus.Cancelled, OrderStatus.Confirmed)).toBe(false);
      });

      it('should not allow OutForDelivery → Cancelled transition', () => {
        expect(canTransitionTo(OrderStatus.OutForDelivery, OrderStatus.Cancelled)).toBe(false);
      });
    });

    describe('transitionStatus', () => {
      it('should transition from Pending to Confirmed', () => {
        const updated = transitionStatus(baseOrder, OrderStatus.Confirmed);

        expect(updated.status).toBe(OrderStatus.Confirmed);
        expect(updated.updatedAt.getTime()).toBeGreaterThanOrEqual(
          baseOrder.updatedAt.getTime()
        );
      });

      it('should transition through full happy path: Pending → Confirmed → Preparing', () => {
        const confirmed = transitionStatus(baseOrder, OrderStatus.Confirmed);
        expect(confirmed.status).toBe(OrderStatus.Confirmed);

        const preparing = transitionStatus(confirmed, OrderStatus.Preparing);
        expect(preparing.status).toBe(OrderStatus.Preparing);
      });

      it('should transition through full delivery flow', () => {
        let order = baseOrder;
        order = transitionStatus(order, OrderStatus.Confirmed);
        order = transitionStatus(order, OrderStatus.Preparing);
        order = transitionStatus(order, OrderStatus.OutForDelivery);
        order = transitionStatus(order, OrderStatus.Delivered);

        expect(order.status).toBe(OrderStatus.Delivered);
      });

      it('should allow cancellation from Pending', () => {
        const cancelled = transitionStatus(baseOrder, OrderStatus.Cancelled);
        expect(cancelled.status).toBe(OrderStatus.Cancelled);
      });

      it('should allow cancellation from Confirmed', () => {
        const confirmed = transitionStatus(baseOrder, OrderStatus.Confirmed);
        const cancelled = transitionStatus(confirmed, OrderStatus.Cancelled);
        expect(cancelled.status).toBe(OrderStatus.Cancelled);
      });

      it('should allow cancellation from Preparing', () => {
        const confirmed = transitionStatus(baseOrder, OrderStatus.Confirmed);
        const preparing = transitionStatus(confirmed, OrderStatus.Preparing);
        const cancelled = transitionStatus(preparing, OrderStatus.Cancelled);
        expect(cancelled.status).toBe(OrderStatus.Cancelled);
      });

      it('should update updatedAt timestamp on transition', () => {
        const beforeTransition = new Date();
        const updated = transitionStatus(baseOrder, OrderStatus.Confirmed);

        expect(updated.updatedAt.getTime()).toBeGreaterThanOrEqual(beforeTransition.getTime());
        expect(updated.updatedAt.getTime()).toBeGreaterThanOrEqual(
          baseOrder.updatedAt.getTime()
        );
      });

      it('should preserve all other order properties during transition', () => {
        const updated = transitionStatus(baseOrder, OrderStatus.Confirmed);

        expect(updated.orderId).toEqual(baseOrder.orderId);
        expect(updated.customerId).toEqual(baseOrder.customerId);
        expect(updated.restaurantId).toEqual(baseOrder.restaurantId);
        expect(updated.items).toEqual(baseOrder.items);
        expect(updated.totalAmount).toEqual(baseOrder.totalAmount);
        expect(updated.createdAt).toEqual(baseOrder.createdAt);
      });

      it('should throw InvalidStatusTransitionError for invalid transition', () => {
        expect(() => transitionStatus(baseOrder, OrderStatus.Preparing)).toThrow(
          InvalidStatusTransitionError
        );
      });

      it('should throw error when transitioning from Delivered', () => {
        let order = baseOrder;
        order = transitionStatus(order, OrderStatus.Confirmed);
        order = transitionStatus(order, OrderStatus.Preparing);
        order = transitionStatus(order, OrderStatus.OutForDelivery);
        order = transitionStatus(order, OrderStatus.Delivered);

        expect(() => transitionStatus(order, OrderStatus.Pending)).toThrow(
          InvalidStatusTransitionError
        );
      });

      it('should throw error when transitioning from Cancelled', () => {
        const cancelled = transitionStatus(baseOrder, OrderStatus.Cancelled);

        expect(() => transitionStatus(cancelled, OrderStatus.Confirmed)).toThrow(
          InvalidStatusTransitionError
        );
      });

      it('should not allow skipping statuses', () => {
        expect(() => transitionStatus(baseOrder, OrderStatus.OutForDelivery)).toThrow(
          InvalidStatusTransitionError
        );
      });
    });
  });

  describe('Validation Rules', () => {
    it('should enforce minimum item quantity of 1', () => {
      const items: OrderItem[] = [createMockItem('item-1', 'Burger', 10.5, 1)];
      const order = createOrder(mockOrderId, mockCustomerId, mockRestaurantId, items);

      expect(order.items[0].quantity).toBeGreaterThan(0);
    });

    it('should handle large quantities', () => {
      const items: OrderItem[] = [createMockItem('item-1', 'Napkin', 0.05, 100)];
      const order = createOrder(mockOrderId, mockCustomerId, mockRestaurantId, items);

      expect(order.totalAmount).toEqual({ amount: 5.0, currency: 'USD' });
    });

    it('should handle decimal prices correctly', () => {
      const items: OrderItem[] = [
        createMockItem('item-1', 'Coffee', 3.99, 1),
        createMockItem('item-2', 'Muffin', 2.49, 1),
      ];
      const order = createOrder(mockOrderId, mockCustomerId, mockRestaurantId, items);

      expect(order.totalAmount.amount).toBeCloseTo(6.48, 2);
    });

    it('should maintain order immutability', () => {
      const items: OrderItem[] = [createMockItem('item-1', 'Burger', 10.5, 1)];
      const order = createOrder(mockOrderId, mockCustomerId, mockRestaurantId, items);
      const transitioned = transitionStatus(order, OrderStatus.Confirmed);

      expect(order.status).toBe(OrderStatus.Pending);
      expect(transitioned.status).toBe(OrderStatus.Confirmed);
      expect(order).not.toBe(transitioned);
    });
  });

  describe('InvalidStatusTransitionError', () => {
    it('should have correct error message', () => {
      const error = new InvalidStatusTransitionError(
        OrderStatus.Pending,
        OrderStatus.Preparing
      );

      expect(error.message).toBe('Invalid status transition from Pending to Preparing');
      expect(error.name).toBe('InvalidStatusTransitionError');
    });

    it('should be throwable and catchable', () => {
      const items: OrderItem[] = [createMockItem('item-1', 'Burger', 10.5, 1)];
      const order = createOrder(mockOrderId, mockCustomerId, mockRestaurantId, items);

      try {
        transitionStatus(order, OrderStatus.Preparing);
        expect.fail('Should have thrown error');
      } catch (error) {
        expect(error).toBeInstanceOf(InvalidStatusTransitionError);
        if (error instanceof InvalidStatusTransitionError) {
          expect(error.message).toContain('Invalid status transition');
        }
      }
    });
  });
});
