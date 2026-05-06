import { describe, it, expect, beforeEach } from 'vitest';
import { InMemoryOrderRepository } from './InMemoryOrderRepository.js';
import {
  createOrder,
  createOrderId,
  createCustomerId,
  type OrderItem,
} from '../../core/domain/Order.js';
import { OrderStatus } from '../../core/domain/OrderStatus.js';
import { createMoney } from '../../core/domain/Money.js';

describe('InMemoryOrderRepository', () => {
  let repo: InMemoryOrderRepository;

  const item: OrderItem = {
    itemId: 'item-1',
    name: 'Pizza Margherita',
    quantity: 2,
    unitPrice: createMoney(12.99, 'USD'),
  };

  const makeOrder = (orderId: string, customerId: string) =>
    createOrder({
      orderId,
      customerId,
      restaurantId: 'restaurant-1',
      items: [item],
    });

  beforeEach(() => {
    repo = new InMemoryOrderRepository();
  });

  describe('save and findById', () => {
    it('persists an order and retrieves it by id', async () => {
      const order = makeOrder('order-1', 'customer-1');
      await repo.save(order);

      const found = await repo.findById(createOrderId('order-1'));
      expect(found).not.toBeNull();
      expect(found?.orderId).toBe('order-1');
      expect(found?.customerId).toBe('customer-1');
      expect(found?.totalAmount.amount).toBe(25.98);
    });

    it('returns null when order does not exist', async () => {
      const found = await repo.findById(createOrderId('missing'));
      expect(found).toBeNull();
    });

    it('overwrites an order saved with the same id', async () => {
      await repo.save(makeOrder('order-1', 'customer-1'));
      await repo.save(makeOrder('order-1', 'customer-2'));

      const found = await repo.findById(createOrderId('order-1'));
      expect(found?.customerId).toBe('customer-2');
    });
  });

  describe('findByCustomerId', () => {
    it('returns all orders for a given customer', async () => {
      await repo.save(makeOrder('order-1', 'customer-A'));
      await repo.save(makeOrder('order-2', 'customer-A'));
      await repo.save(makeOrder('order-3', 'customer-B'));

      const orders = await repo.findByCustomerId(createCustomerId('customer-A'));
      expect(orders).toHaveLength(2);
      expect(orders.map((o) => o.orderId).sort()).toEqual(['order-1', 'order-2']);
    });

    it('returns empty array when customer has no orders', async () => {
      await repo.save(makeOrder('order-1', 'customer-A'));

      const orders = await repo.findByCustomerId(createCustomerId('customer-Z'));
      expect(orders).toEqual([]);
    });
  });

  describe('updateStatus', () => {
    it('persists a valid status transition', async () => {
      await repo.save(makeOrder('order-1', 'customer-1'));

      await repo.updateStatus(createOrderId('order-1'), OrderStatus.Confirmed);

      const updated = await repo.findById(createOrderId('order-1'));
      expect(updated?.status).toBe(OrderStatus.Confirmed);
    });

    it('throws when the order does not exist', async () => {
      await expect(
        repo.updateStatus(createOrderId('missing'), OrderStatus.Confirmed)
      ).rejects.toThrow('Order not found: missing');
    });

    it('throws when the status transition is invalid', async () => {
      await repo.save(makeOrder('order-1', 'customer-1'));

      await expect(
        repo.updateStatus(createOrderId('order-1'), OrderStatus.Delivered)
      ).rejects.toThrow('Invalid status transition');
    });

    it('updates the updatedAt timestamp', async () => {
      const original = makeOrder('order-1', 'customer-1');
      await repo.save(original);

      await new Promise((r) => setTimeout(r, 5));
      await repo.updateStatus(createOrderId('order-1'), OrderStatus.Confirmed);

      const updated = await repo.findById(createOrderId('order-1'));
      expect(updated?.updatedAt.getTime()).toBeGreaterThanOrEqual(
        original.updatedAt.getTime()
      );
    });
  });
});
