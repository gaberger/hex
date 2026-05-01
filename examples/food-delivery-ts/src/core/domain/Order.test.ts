import { describe, it, expect } from 'vitest';
import {
  Order,
  OrderId,
  CustomerId,
  RestaurantId,
  ItemId,
  OrderItem,
  createOrder,
  transitionStatus,
  calculateTotal,
  CreateOrderParams,
} from './Order.js';
import { OrderStatus } from './OrderStatus.js';
import { Money } from './Money.js';

describe('Order', () => {
  describe('ID factory functions', () => {
    it('should create OrderId from non-empty string', () => {
      const id = OrderId('order-123');
      expect(id).toBe('order-123');
    });

    it('should throw error for empty OrderId', () => {
      expect(() => OrderId('')).toThrow('OrderId cannot be empty');
    });

    it('should throw error for whitespace-only OrderId', () => {
      expect(() => OrderId('   ')).toThrow('OrderId cannot be empty');
    });

    it('should create CustomerId from non-empty string', () => {
      const id = CustomerId('customer-456');
      expect(id).toBe('customer-456');
    });

    it('should throw error for empty CustomerId', () => {
      expect(() => CustomerId('')).toThrow('CustomerId cannot be empty');
    });

    it('should create RestaurantId from non-empty string', () => {
      const id = RestaurantId('restaurant-789');
      expect(id).toBe('restaurant-789');
    });

    it('should throw error for empty RestaurantId', () => {
      expect(() => RestaurantId('')).toThrow('RestaurantId cannot be empty');
    });

    it('should create ItemId from non-empty string', () => {
      const id = ItemId('item-101');
      expect(id).toBe('item-101');
    });

    it('should throw error for empty ItemId', () => {
      expect(() => ItemId('')).toThrow('ItemId cannot be empty');
    });
  });

  describe('createOrder', () => {
    const sampleItems: OrderItem[] = [
      {
        itemId: ItemId('item-1'),
        name: 'Burger',
        quantity: 2,
        unitPrice: Money.of(10, 'USD'),
      },
      {
        itemId: ItemId('item-2'),
        name: 'Fries',
        quantity: 1,
        unitPrice: Money.of(5, 'USD'),
      },
    ];

    it('should create order with valid parameters', () => {
      const params: CreateOrderParams = {
        customerId: CustomerId('customer-1'),
        restaurantId: RestaurantId('restaurant-1'),
        items: sampleItems,
      };

      const order = createOrder(params);

      expect(order.customerId).toBe('customer-1');
      expect(order.restaurantId).toBe('restaurant-1');
      expect(order.items).toHaveLength(2);
      expect(order.status).toBe(OrderStatus.Pending);
      expect(order.totalAmount.amount).toBe(25); // 2*10 + 1*5
      expect(order.totalAmount.currency).toBe('USD');
      expect(order.id).toBeTruthy();
      expect(order.createdAt).toBeInstanceOf(Date);
      expect(order.updatedAt).toBeInstanceOf(Date);
      expect(order.createdAt.getTime()).toBe(order.updatedAt.getTime());
    });

    it('should throw error when items array is empty', () => {
      const params: CreateOrderParams = {
        customerId: CustomerId('customer-1'),
        restaurantId: RestaurantId('restaurant-1'),
        items: [],
      };

      expect(() => createOrder(params)).toThrow(
        'Order must contain at least one item'
      );
    });

    it('should throw error when items have different currencies', () => {
      const params: CreateOrderParams = {
        customerId: CustomerId('customer-1'),
        restaurantId: RestaurantId('restaurant-1'),
        items: [
          {
            itemId: ItemId('item-1'),
            name: 'Burger',
            quantity: 1,
            unitPrice: Money.of(10, 'USD'),
          },
          {
            itemId: ItemId('item-2'),
            name: 'Pizza',
            quantity: 1,
            unitPrice: Money.of(15, 'EUR'),
          },
        ],
      };

      expect(() => createOrder(params)).toThrow(
        'All items must have the same currency'
      );
    });

    it('should throw error when item has non-positive quantity', () => {
      const params: CreateOrderParams = {
        customerId: CustomerId('customer-1'),
        restaurantId: RestaurantId('restaurant-1'),
        items: [
          {
            itemId: ItemId('item-1'),
            name: 'Burger',
            quantity: 0,
            unitPrice: Money.of(10, 'USD'),
          },
        ],
      };

      expect(() => createOrder(params)).toThrow(
        'All items must have positive quantity'
      );
    });

    it('should throw error when item has negative quantity', () => {
      const params: CreateOrderParams = {
        customerId: CustomerId('customer-1'),
        restaurantId: RestaurantId('restaurant-1'),
        items: [
          {
            itemId: ItemId('item-1'),
            name: 'Burger',
            quantity: -1,
            unitPrice: Money.of(10, 'USD'),
          },
        ],
      };

      expect(() => createOrder(params)).toThrow(
        'All items must have positive quantity'
      );
    });

    it('should calculate correct total for single item', () => {
      const params: CreateOrderParams = {
        customerId: CustomerId('customer-1'),
        restaurantId: RestaurantId('restaurant-1'),
        items: [
          {
            itemId: ItemId('item-1'),
            name: 'Burger',
            quantity: 3,
            unitPrice: Money.of(12.5, 'USD'),
          },
        ],
      };

      const order = createOrder(params);
      expect(order.totalAmount.amount).toBe(37.5);
    });

    it('should calculate correct total for multiple items', () => {
      const params: CreateOrderParams = {
        customerId: CustomerId('customer-1'),
        restaurantId: RestaurantId('restaurant-1'),
        items: [
          {
            itemId: ItemId('item-1'),
            name: 'Burger',
            quantity: 2,
            unitPrice: Money.of(10, 'USD'),
          },
          {
            itemId: ItemId('item-2'),
            name: 'Fries',
            quantity: 3,
            unitPrice: Money.of(4, 'USD'),
          },
          {
            itemId: ItemId('item-3'),
            name: 'Drink',
            quantity: 1,
            unitPrice: Money.of(2.5, 'USD'),
          },
        ],
      };

      const order = createOrder(params);
      expect(order.totalAmount.amount).toBe(34.5); // 20 + 12 + 2.5
    });
  });

  describe('transitionStatus', () => {
    const sampleOrder: Order = {
      id: OrderId('order-1'),
      customerId: CustomerId('customer-1'),
      restaurantId: RestaurantId('restaurant-1'),
      items: [
        {
          itemId: ItemId('item-1'),
          name: 'Burger',
          quantity: 1,
          unitPrice: Money.of(10, 'USD'),
        },
      ],
      status: OrderStatus.Pending,
      totalAmount: Money.of(10, 'USD'),
      createdAt: new Date('2026-01-01T10:00:00Z'),
      updatedAt: new Date('2026-01-01T10:00:00Z'),
    };

    it('should transition from Pending to Confirmed', () => {
      const updated = transitionStatus(sampleOrder, OrderStatus.Confirmed);
      expect(updated.status).toBe(OrderStatus.Confirmed);
      expect(updated.updatedAt.getTime()).toBeGreaterThan(
        sampleOrder.updatedAt.getTime()
      );
      expect(updated.id).toBe(sampleOrder.id);
      expect(updated.items).toBe(sampleOrder.items);
    });

    it('should transition from Pending to Cancelled', () => {
      const updated = transitionStatus(sampleOrder, OrderStatus.Cancelled);
      expect(updated.status).toBe(OrderStatus.Cancelled);
    });

    it('should transition from Confirmed to Preparing', () => {
      const confirmedOrder = { ...sampleOrder, status: OrderStatus.Confirmed };
      const updated = transitionStatus(confirmedOrder, OrderStatus.Preparing);
      expect(updated.status).toBe(OrderStatus.Preparing);
    });

    it('should transition from Confirmed to Cancelled', () => {
      const confirmedOrder = { ...sampleOrder, status: OrderStatus.Confirmed };
      const updated = transitionStatus(confirmedOrder, OrderStatus.Cancelled);
      expect(updated.status).toBe(OrderStatus.Cancelled);
    });

    it('should transition from Preparing to OutForDelivery', () => {
      const preparingOrder = { ...sampleOrder, status: OrderStatus.Preparing };
      const updated = transitionStatus(preparingOrder, OrderStatus.OutForDelivery);
      expect(updated.status).toBe(OrderStatus.OutForDelivery);
    });

    it('should transition from Preparing to Cancelled', () => {
      const preparingOrder = { ...sampleOrder, status: OrderStatus.Preparing };
      const updated = transitionStatus(preparingOrder, OrderStatus.Cancelled);
      expect(updated.status).toBe(OrderStatus.Cancelled);
    });

    it('should transition from OutForDelivery to Delivered', () => {
      const outForDeliveryOrder = { ...sampleOrder, status: OrderStatus.OutForDelivery };
      const updated = transitionStatus(outForDeliveryOrder, OrderStatus.Delivered);
      expect(updated.status).toBe(OrderStatus.Delivered);
    });

    it('should throw error for invalid transition from Pending to Preparing', () => {
      expect(() => transitionStatus(sampleOrder, OrderStatus.Preparing)).toThrow(
        'Invalid status transition from Pending to Preparing'
      );
    });

    it('should throw error for invalid transition from Pending to OutForDelivery', () => {
      expect(() =>
        transitionStatus(sampleOrder, OrderStatus.OutForDelivery)
      ).toThrow('Invalid status transition from Pending to OutForDelivery');
    });

    it('should throw error for invalid transition from Pending to Delivered', () => {
      expect(() => transitionStatus(sampleOrder, OrderStatus.Delivered)).toThrow(
        'Invalid status transition from Pending to Delivered'
      );
    });

    it('should throw error for transition from Delivered status', () => {
      const deliveredOrder = { ...sampleOrder, status: OrderStatus.Delivered };
      expect(() =>
        transitionStatus(deliveredOrder, OrderStatus.Confirmed)
      ).toThrow('Invalid status transition from Delivered to Confirmed');
    });

    it('should throw error for transition from Cancelled status', () => {
      const cancelledOrder = { ...sampleOrder, status: OrderStatus.Cancelled };
      expect(() =>
        transitionStatus(cancelledOrder, OrderStatus.Confirmed)
      ).toThrow('Invalid status transition from Cancelled to Confirmed');
    });

    it('should not allow transition from Confirmed to OutForDelivery', () => {
      const confirmedOrder = { ...sampleOrder, status: OrderStatus.Confirmed };
      expect(() =>
        transitionStatus(confirmedOrder, OrderStatus.OutForDelivery)
      ).toThrow('Invalid status transition from Confirmed to OutForDelivery');
    });

    it('should preserve original order properties during transition', () => {
      const updated = transitionStatus(sampleOrder, OrderStatus.Confirmed);
      expect(updated.id).toBe(sampleOrder.id);
      expect(updated.customerId).toBe(sampleOrder.customerId);
      expect(updated.restaurantId).toBe(sampleOrder.restaurantId);
      expect(updated.items).toBe(sampleOrder.items);
      expect(updated.totalAmount).toBe(sampleOrder.totalAmount);
      expect(updated.createdAt).toBe(sampleOrder.createdAt);
    });
  });

  describe('calculateTotal', () => {
    it('should calculate total for single item', () => {
      const items: OrderItem[] = [
        {
          itemId: ItemId('item-1'),
          name: 'Burger',
          quantity: 2,
          unitPrice: Money.of(10, 'USD'),
        },
      ];

      const total = calculateTotal(items);
      expect(total.amount).toBe(20);
      expect(total.currency).toBe('USD');
    });

    it('should calculate total for multiple items', () => {
      const items: OrderItem[] = [
        {
          itemId: ItemId('item-1'),
          name: 'Burger',
          quantity: 2,
          unitPrice: Money.of(10, 'USD'),
        },
        {
          itemId: ItemId('item-2'),
          name: 'Fries',
          quantity: 3,
          unitPrice: Money.of(5, 'USD'),
        },
      ];

      const total = calculateTotal(items);
      expect(total.amount).toBe(35); // 20 + 15
      expect(total.currency).toBe('USD');
    });

    it('should throw error for empty items list', () => {
      expect(() => calculateTotal([])).toThrow(
        'Cannot calculate total for empty items list'
      );
    });

    it('should handle decimal prices correctly', () => {
      const items: OrderItem[] = [
        {
          itemId: ItemId('item-1'),
          name: 'Coffee',
          quantity: 2,
          unitPrice: Money.of(3.5, 'USD'),
        },
        {
          itemId: ItemId('item-2'),
          name: 'Muffin',
          quantity: 1,
          unitPrice: Money.of(4.25, 'USD'),
        },
      ];

      const total = calculateTotal(items);
      expect(total.amount).toBe(11.25); // 7 + 4.25
      expect(total.currency).toBe('USD');
    });
  });

  describe('Order immutability', () => {
    it('should not mutate original order when transitioning status', () => {
      const originalOrder = createOrder({
        customerId: CustomerId('customer-1'),
        restaurantId: RestaurantId('restaurant-1'),
        items: [
          {
            itemId: ItemId('item-1'),
            name: 'Burger',
            quantity: 1,
            unitPrice: Money.of(10, 'USD'),
          },
        ],
      });

      const originalStatus = originalOrder.status;
      const originalUpdatedAt = originalOrder.updatedAt;

      const updated = transitionStatus(originalOrder, OrderStatus.Confirmed);

      expect(originalOrder.status).toBe(originalStatus);
      expect(originalOrder.updatedAt).toBe(originalUpdatedAt);
      expect(updated.status).toBe(OrderStatus.Confirmed);
      expect(updated.status).not.toBe(originalOrder.status);
    });
  });

  describe('Full order lifecycle', () => {
    it('should support complete order flow from creation to delivery', () => {
      // Create order
      const order = createOrder({
        customerId: CustomerId('customer-1'),
        restaurantId: RestaurantId('restaurant-1'),
        items: [
          {
            itemId: ItemId('item-1'),
            name: 'Pizza',
            quantity: 2,
            unitPrice: Money.of(15, 'USD'),
          },
        ],
      });
      expect(order.status).toBe(OrderStatus.Pending);

      // Confirm order
      const confirmed = transitionStatus(order, OrderStatus.Confirmed);
      expect(confirmed.status).toBe(OrderStatus.Confirmed);

      // Start preparing
      const preparing = transitionStatus(confirmed, OrderStatus.Preparing);
      expect(preparing.status).toBe(OrderStatus.Preparing);

      // Out for delivery
      const outForDelivery = transitionStatus(preparing, OrderStatus.OutForDelivery);
      expect(outForDelivery.status).toBe(OrderStatus.OutForDelivery);

      // Delivered
      const delivered = transitionStatus(outForDelivery, OrderStatus.Delivered);
      expect(delivered.status).toBe(OrderStatus.Delivered);

      // Verify total remains correct
      expect(delivered.totalAmount.amount).toBe(30);
    });

    it('should support order cancellation at Pending stage', () => {
      const order = createOrder({
        customerId: CustomerId('customer-1'),
        restaurantId: RestaurantId('restaurant-1'),
        items: [
          {
            itemId: ItemId('item-1'),
            name: 'Burger',
            quantity: 1,
            unitPrice: Money.of(10, 'USD'),
          },
        ],
      });

      const cancelled = transitionStatus(order, OrderStatus.Cancelled);
      expect(cancelled.status).toBe(OrderStatus.Cancelled);
    });

    it('should support order cancellation at Confirmed stage', () => {
      const order = createOrder({
        customerId: CustomerId('customer-1'),
        restaurantId: RestaurantId('restaurant-1'),
        items: [
          {
            itemId: ItemId('item-1'),
            name: 'Burger',
            quantity: 1,
            unitPrice: Money.of(10, 'USD'),
          },
        ],
      });

      const confirmed = transitionStatus(order, OrderStatus.Confirmed);
      const cancelled = transitionStatus(confirmed, OrderStatus.Cancelled);
      expect(cancelled.status).toBe(OrderStatus.Cancelled);
    });

    it('should support order cancellation at Preparing stage', () => {
      const order = createOrder({
        customerId: CustomerId('customer-1'),
        restaurantId: RestaurantId('restaurant-1'),
        items: [
          {
            itemId: ItemId('item-1'),
            name: 'Burger',
            quantity: 1,
            unitPrice: Money.of(10, 'USD'),
          },
        ],
      });

      const confirmed = transitionStatus(order, OrderStatus.Confirmed);
      const preparing = transitionStatus(confirmed, OrderStatus.Preparing);
      const cancelled = transitionStatus(preparing, OrderStatus.Cancelled);
      expect(cancelled.status).toBe(OrderStatus.Cancelled);
    });
  });
});
