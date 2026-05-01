import { describe, it, expect, beforeEach } from 'vitest';
import { InMemoryOrderRepository } from './InMemoryOrderRepository.js';
import { createOrder } from '../../core/domain/Order.js';
import { OrderStatus } from '../../core/domain/OrderStatus.js';
import type { OrderId, CustomerId, RestaurantId, OrderItem } from '../../core/domain/Order.js';

describe('InMemoryOrderRepository', () => {
  let repository: InMemoryOrderRepository;

  beforeEach(() => {
    repository = new InMemoryOrderRepository();
  });

  const createTestOrder = (orderId: string, customerId: string) => {
    const items: OrderItem[] = [
      {
        itemId: 'item-1',
        name: 'Burger',
        price: { amount: 10.0, currency: 'USD' },
        quantity: 2,
      },
      {
        itemId: 'item-2',
        name: 'Fries',
        price: { amount: 3.5, currency: 'USD' },
        quantity: 1,
      },
    ];

    return createOrder(
      { value: orderId } as OrderId,
      { value: customerId } as CustomerId,
      { value: 'restaurant-1' } as RestaurantId,
      items
    );
  };

  describe('save and findById', () => {
    it('should save an order and retrieve it by id', async () => {
      const order = createTestOrder('order-1', 'customer-1');

      await repository.save(order);
      const retrieved = await repository.findById({ value: 'order-1' });

      expect(retrieved).not.toBeNull();
      expect(retrieved?.orderId.value).toBe('order-1');
      expect(retrieved?.customerId.value).toBe('customer-1');
      expect(retrieved?.items.length).toBe(2);
      expect(retrieved?.totalAmount.amount).toBe(23.5);
    });

    it('should return null when order does not exist', async () => {
      const retrieved = await repository.findById({ value: 'nonexistent' });
      expect(retrieved).toBeNull();
    });

    it('should overwrite existing order on save', async () => {
      const order = createTestOrder('order-1', 'customer-1');
      await repository.save(order);

      const updatedOrder = { ...order, status: OrderStatus.Confirmed };
      await repository.save(updatedOrder);

      const retrieved = await repository.findById({ value: 'order-1' });
      expect(retrieved?.status).toBe(OrderStatus.Confirmed);
    });
  });

  describe('findByCustomerId', () => {
    it('should return all orders for a customer', async () => {
      const order1 = createTestOrder('order-1', 'customer-1');
      const order2 = createTestOrder('order-2', 'customer-1');
      const order3 = createTestOrder('order-3', 'customer-2');

      await repository.save(order1);
      await repository.save(order2);
      await repository.save(order3);

      const customerOrders = await repository.findByCustomerId({ value: 'customer-1' });

      expect(customerOrders.length).toBe(2);
      expect(customerOrders.map((o) => o.orderId.value).sort()).toEqual(['order-1', 'order-2']);
    });

    it('should return empty array when customer has no orders', async () => {
      const orders = await repository.findByCustomerId({ value: 'customer-nonexistent' });
      expect(orders).toEqual([]);
    });

    it('should return correct orders after updates', async () => {
      const order1 = createTestOrder('order-1', 'customer-1');
      const order2 = createTestOrder('order-2', 'customer-2');

      await repository.save(order1);
      await repository.save(order2);

      // Update order1 to have a different status (but same customer)
      const updatedOrder1 = { ...order1, status: OrderStatus.Confirmed };
      await repository.save(updatedOrder1);

      const customerOrders = await repository.findByCustomerId({ value: 'customer-1' });

      expect(customerOrders.length).toBe(1);
      expect(customerOrders[0].orderId.value).toBe('order-1');
      expect(customerOrders[0].status).toBe(OrderStatus.Confirmed);
    });
  });

  describe('updateStatus', () => {
    it('should update order status and persist changes', async () => {
      const order = createTestOrder('order-1', 'customer-1');
      await repository.save(order);

      await repository.updateStatus({ value: 'order-1' }, OrderStatus.Confirmed);

      const retrieved = await repository.findById({ value: 'order-1' });
      expect(retrieved?.status).toBe(OrderStatus.Confirmed);
    });

    it('should throw when order does not exist', async () => {
      await expect(
        repository.updateStatus({ value: 'nonexistent' }, OrderStatus.Confirmed)
      ).rejects.toThrow('Order not found: nonexistent');
    });

    it('should update updatedAt timestamp', async () => {
      const order = createTestOrder('order-1', 'customer-1');
      await repository.save(order);

      const originalUpdatedAt = order.updatedAt.getTime();

      // Small delay to ensure timestamp difference
      await new Promise((resolve) => setTimeout(resolve, 10));

      await repository.updateStatus({ value: 'order-1' }, OrderStatus.Confirmed);

      const retrieved = await repository.findById({ value: 'order-1' });
      expect(retrieved?.updatedAt.getTime()).toBeGreaterThan(originalUpdatedAt);
    });

    it('should preserve all other order properties', async () => {
      const order = createTestOrder('order-1', 'customer-1');
      await repository.save(order);

      await repository.updateStatus({ value: 'order-1' }, OrderStatus.Confirmed);

      const retrieved = await repository.findById({ value: 'order-1' });
      expect(retrieved?.orderId.value).toBe('order-1');
      expect(retrieved?.customerId.value).toBe('customer-1');
      expect(retrieved?.items.length).toBe(2);
      expect(retrieved?.totalAmount.amount).toBe(23.5);
      expect(retrieved?.createdAt).toEqual(order.createdAt);
    });

    it('should handle multiple status transitions correctly', async () => {
      const order = createTestOrder('order-1', 'customer-1');
      await repository.save(order);

      await repository.updateStatus({ value: 'order-1' }, OrderStatus.Confirmed);
      await repository.updateStatus({ value: 'order-1' }, OrderStatus.Preparing);
      await repository.updateStatus({ value: 'order-1' }, OrderStatus.OutForDelivery);

      const retrieved = await repository.findById({ value: 'order-1' });
      expect(retrieved?.status).toBe(OrderStatus.OutForDelivery);
    });
  });

  describe('edge cases', () => {
    it('should handle multiple orders with different customers', async () => {
      const order1 = createTestOrder('order-1', 'customer-1');
      const order2 = createTestOrder('order-2', 'customer-2');
      const order3 = createTestOrder('order-3', 'customer-3');

      await repository.save(order1);
      await repository.save(order2);
      await repository.save(order3);

      const customer1Orders = await repository.findByCustomerId({ value: 'customer-1' });
      const customer2Orders = await repository.findByCustomerId({ value: 'customer-2' });
      const customer3Orders = await repository.findByCustomerId({ value: 'customer-3' });

      expect(customer1Orders.length).toBe(1);
      expect(customer2Orders.length).toBe(1);
      expect(customer3Orders.length).toBe(1);
    });

    it('should handle rapid concurrent saves', async () => {
      const order1 = createTestOrder('order-1', 'customer-1');
      const order2 = createTestOrder('order-2', 'customer-1');
      const order3 = createTestOrder('order-3', 'customer-1');

      await Promise.all([
        repository.save(order1),
        repository.save(order2),
        repository.save(order3),
      ]);

      const orders = await repository.findByCustomerId({ value: 'customer-1' });
      expect(orders.length).toBe(3);
    });
  });
});
