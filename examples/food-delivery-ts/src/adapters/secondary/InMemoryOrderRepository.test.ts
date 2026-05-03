import { describe, it, expect, beforeEach } from 'vitest';
import { InMemoryOrderRepository } from './InMemoryOrderRepository.js';
import { createOrder } from '../../core/domain/Order.js';
import { OrderStatus } from '../../core/domain/OrderStatus.js';
import type { OrderItem } from '../../core/domain/Order.js';

describe('InMemoryOrderRepository', () => {
  let repo: InMemoryOrderRepository;
  beforeEach(() => {
    repo = new InMemoryOrderRepository();
  });

  const makeOrder = (orderId: string, customerId: string) => {
    const items: OrderItem[] = [
      { itemId: 'item-1', name: 'Burger', price: { amount: 10.0, currency: 'USD' }, quantity: 2 },
      { itemId: 'item-2', name: 'Fries', price: { amount: 3.5, currency: 'USD' }, quantity: 1 },
    ];
    return createOrder({ orderId, customerId, restaurantId: 'restaurant-1', items });
  };

  describe('save and findById', () => {
    it('saves and retrieves by id', async () => {
      await repo.save(makeOrder('order-1', 'customer-1'));
      const got = await repo.findById('order-1');
      expect(got).not.toBeNull();
      expect(got?.orderId).toBe('order-1');
      expect(got?.customerId).toBe('customer-1');
      expect(got?.items.length).toBe(2);
      expect(got?.totalAmount.amount).toBe(23.5);
    });
    it('returns null for missing', async () => {
      expect(await repo.findById('nope')).toBeNull();
    });
    it('overwrites on save', async () => {
      const o = makeOrder('order-1', 'customer-1');
      await repo.save(o);
      await repo.save({ ...o, status: OrderStatus.Confirmed });
      expect((await repo.findById('order-1'))?.status).toBe(OrderStatus.Confirmed);
    });
  });

  describe('findByCustomerId', () => {
    it('returns all orders for a customer', async () => {
      await repo.save(makeOrder('order-1', 'customer-1'));
      await repo.save(makeOrder('order-2', 'customer-1'));
      await repo.save(makeOrder('order-3', 'customer-2'));
      const got = await repo.findByCustomerId('customer-1');
      expect(got.length).toBe(2);
      expect(got.map(o => o.orderId).sort()).toEqual(['order-1', 'order-2']);
    });
    it('empty for unknown customer', async () => {
      expect(await repo.findByCustomerId('nope')).toEqual([]);
    });
    it('correct after updates', async () => {
      const o1 = makeOrder('order-1', 'customer-1');
      await repo.save(o1);
      await repo.save(makeOrder('order-2', 'customer-2'));
      await repo.save({ ...o1, status: OrderStatus.Confirmed });
      const got = await repo.findByCustomerId('customer-1');
      expect(got.length).toBe(1);
      expect(got[0].orderId).toBe('order-1');
      expect(got[0].status).toBe(OrderStatus.Confirmed);
    });
  });

  describe('updateStatus', () => {
    it('updates status', async () => {
      await repo.save(makeOrder('order-1', 'customer-1'));
      await repo.updateStatus('order-1', OrderStatus.Confirmed);
      expect((await repo.findById('order-1'))?.status).toBe(OrderStatus.Confirmed);
    });
    it('throws on missing', async () => {
      await expect(repo.updateStatus('nope', OrderStatus.Confirmed)).rejects.toThrow(
        'Order not found: nope'
      );
    });
    it('updates updatedAt', async () => {
      const o = makeOrder('order-1', 'customer-1');
      await repo.save(o);
      const before = o.updatedAt.getTime();
      await new Promise(r => setTimeout(r, 10));
      await repo.updateStatus('order-1', OrderStatus.Confirmed);
      expect((await repo.findById('order-1'))!.updatedAt.getTime()).toBeGreaterThan(before);
    });
    it('preserves other props', async () => {
      const o = makeOrder('order-1', 'customer-1');
      await repo.save(o);
      await repo.updateStatus('order-1', OrderStatus.Confirmed);
      const got = await repo.findById('order-1');
      expect(got?.orderId).toBe('order-1');
      expect(got?.customerId).toBe('customer-1');
      expect(got?.items.length).toBe(2);
      expect(got?.totalAmount.amount).toBe(23.5);
      expect(got?.createdAt).toEqual(o.createdAt);
    });
    it('multi-step transitions', async () => {
      await repo.save(makeOrder('order-1', 'customer-1'));
      await repo.updateStatus('order-1', OrderStatus.Confirmed);
      await repo.updateStatus('order-1', OrderStatus.Preparing);
      await repo.updateStatus('order-1', OrderStatus.OutForDelivery);
      expect((await repo.findById('order-1'))?.status).toBe(OrderStatus.OutForDelivery);
    });
  });

  describe('edge cases', () => {
    it('multiple customers', async () => {
      await repo.save(makeOrder('order-1', 'customer-1'));
      await repo.save(makeOrder('order-2', 'customer-2'));
      await repo.save(makeOrder('order-3', 'customer-3'));
      expect((await repo.findByCustomerId('customer-1')).length).toBe(1);
      expect((await repo.findByCustomerId('customer-2')).length).toBe(1);
      expect((await repo.findByCustomerId('customer-3')).length).toBe(1);
    });
    it('concurrent saves', async () => {
      await Promise.all([
        repo.save(makeOrder('order-1', 'customer-1')),
        repo.save(makeOrder('order-2', 'customer-1')),
        repo.save(makeOrder('order-3', 'customer-1')),
      ]);
      expect((await repo.findByCustomerId('customer-1')).length).toBe(3);
    });
  });
});
