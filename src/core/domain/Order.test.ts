import { describe, it, expect } from 'bun:test';
import {
  createOrder,
  createItemId,
  transitionOrderStatus,
  updateOrder,
  isValidStatusTransition,
  type OrderItem,
} from './Order.js';
import { OrderStatus } from './OrderStatus.js';
import { createMoney } from './Money.js';

describe('Order Domain', () => {
  const validOrderItem: OrderItem = {
    itemId: createItemId('item-1'),
    name: 'Pizza Margherita',
    quantity: 2,
    unitPrice: createMoney(12.99, 'USD'),
  };

  describe('createOrder', () => {
    it('should create a new order with valid parameters', () => {
      const order = createOrder({
        orderId: 'order-123',
        customerId: 'customer-456',
        restaurantId: 'restaurant-789',
        items: [validOrderItem],
      });

      expect(order.orderId).toBe('order-123');
      expect(order.customerId).toBe('customer-456');
      expect(order.restaurantId).toBe('restaurant-789');
      expect(order.status).toBe(OrderStatus.Pending);
      expect(order.totalAmount.amount).toBe(25.98);
      expect(order.items).toHaveLength(1);
      expect(order.createdAt).toBeInstanceOf(Date);
      expect(order.updatedAt).toBeInstanceOf(Date);
    });

    it('should calculate total amount correctly for multiple items', () => {
      const items: OrderItem[] = [
        { ...validOrderItem, quantity: 2 }, // 25.98
        {
          itemId: 'item-2',
          name: 'Caesar Salad',
          quantity: 1,
          unitPrice: createMoney(8.99, 'USD'),
        }, // 8.99
      ];

      const order = createOrder({
        orderId: 'order-123',
        customerId: 'customer-456',
        restaurantId: 'restaurant-789',
        items,
      });

      expect(order.totalAmount.amount).toBe(34.97);
    });

    it('should throw error if orderId is empty', () => {
      expect(() =>
        createOrder({
          orderId: '',
          customerId: 'customer-456',
          restaurantId: 'restaurant-789',
          items: [validOrderItem],
        })
      ).toThrow('Order ID is required');
    });

    it('should throw error if customerId is empty', () => {
      expect(() =>
        createOrder({
          orderId: 'order-123',
          customerId: '',
          restaurantId: 'restaurant-789',
          items: [validOrderItem],
        })
      ).toThrow('Customer ID is required');
    });

    it('should throw error if restaurantId is empty', () => {
      expect(() =>
        createOrder({
          orderId: 'order-123',
          customerId: 'customer-456',
          restaurantId: '',
          items: [validOrderItem],
        })
      ).toThrow('Restaurant ID is required');
    });

    it('should throw error if items array is empty', () => {
      expect(() =>
        createOrder({
          orderId: 'order-123',
          customerId: 'customer-456',
          restaurantId: 'restaurant-789',
          items: [],
        })
      ).toThrow('Order must contain at least one item');
    });

    it('should throw error if any item has zero or negative quantity', () => {
      const invalidItem = { ...validOrderItem, quantity: 0 };
      expect(() =>
        createOrder({
          orderId: 'order-123',
          customerId: 'customer-456',
          restaurantId: 'restaurant-789',
          items: [invalidItem],
        })
      ).toThrow('must have a positive quantity');
    });
  });

  describe('Status Transitions', () => {
    it('should allow transition from Pending to Confirmed', () => {
      expect(isValidStatusTransition(OrderStatus.Pending, OrderStatus.Confirmed)).toBe(true);
    });

    it('should allow transition from Pending to Cancelled', () => {
      expect(isValidStatusTransition(OrderStatus.Pending, OrderStatus.Cancelled)).toBe(true);
    });

    it('should not allow transition from Pending to Preparing', () => {
      expect(isValidStatusTransition(OrderStatus.Pending, OrderStatus.Preparing)).toBe(false);
    });

    it('should allow full happy path: Pending → Confirmed → Preparing → OutForDelivery → Delivered', () => {
      expect(isValidStatusTransition(OrderStatus.Pending, OrderStatus.Confirmed)).toBe(true);
      expect(isValidStatusTransition(OrderStatus.Confirmed, OrderStatus.Preparing)).toBe(true);
      expect(isValidStatusTransition(OrderStatus.Preparing, OrderStatus.OutForDelivery)).toBe(
        true
      );
      expect(isValidStatusTransition(OrderStatus.OutForDelivery, OrderStatus.Delivered)).toBe(
        true
      );
    });

    it('should not allow transitions from terminal states', () => {
      expect(isValidStatusTransition(OrderStatus.Delivered, OrderStatus.Cancelled)).toBe(false);
      expect(isValidStatusTransition(OrderStatus.Cancelled, OrderStatus.Pending)).toBe(false);
    });
  });

  describe('transitionOrderStatus', () => {
    it('should transition order status when valid', () => {
      const order = createOrder({
        orderId: 'order-123',
        customerId: 'customer-456',
        restaurantId: 'restaurant-789',
        items: [validOrderItem],
      });

      const confirmedOrder = transitionOrderStatus(order, OrderStatus.Confirmed);

      expect(confirmedOrder.status).toBe(OrderStatus.Confirmed);
      expect(confirmedOrder.updatedAt.getTime()).toBeGreaterThanOrEqual(order.updatedAt.getTime());
    });

    it('should throw error when transition is invalid', () => {
      const order = createOrder({
        orderId: 'order-123',
        customerId: 'customer-456',
        restaurantId: 'restaurant-789',
        items: [validOrderItem],
      });

      expect(() => transitionOrderStatus(order, OrderStatus.Preparing)).toThrow(
        'Invalid status transition from Pending to Preparing'
      );
    });

    it('should not allow backwards transitions', () => {
      let order = createOrder({
        orderId: 'order-123',
        customerId: 'customer-456',
        restaurantId: 'restaurant-789',
        items: [validOrderItem],
      });

      order = transitionOrderStatus(order, OrderStatus.Confirmed);
      order = transitionOrderStatus(order, OrderStatus.Preparing);

      expect(() => transitionOrderStatus(order, OrderStatus.Confirmed)).toThrow(
        'Invalid status transition'
      );
    });
  });

  describe('updateOrder', () => {
    it('should update order and recalculate total when items change', () => {
      const order = createOrder({
        orderId: 'order-123',
        customerId: 'customer-456',
        restaurantId: 'restaurant-789',
        items: [validOrderItem],
      });

      const newItems: OrderItem[] = [
        { ...validOrderItem, quantity: 3 }, // 38.97
      ];

      const updatedOrder = updateOrder(order, { items: newItems });

      expect(updatedOrder.items).toEqual(newItems);
      expect(updatedOrder.totalAmount.amount).toBe(38.97);
      expect(updatedOrder.updatedAt.getTime()).toBeGreaterThanOrEqual(
        order.updatedAt.getTime()
      );
    });

    it('should update status without recalculating total', () => {
      const order = createOrder({
        orderId: 'order-123',
        customerId: 'customer-456',
        restaurantId: 'restaurant-789',
        items: [validOrderItem],
      });

      const originalTotal = order.totalAmount.amount;
      const updatedOrder = updateOrder(order, { status: OrderStatus.Confirmed });

      expect(updatedOrder.status).toBe(OrderStatus.Confirmed);
      expect(updatedOrder.totalAmount.amount).toBe(originalTotal);
    });
  });
});
