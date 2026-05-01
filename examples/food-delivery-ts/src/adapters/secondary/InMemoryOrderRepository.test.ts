import { describe, it, expect, beforeEach } from 'vitest';
import { InMemoryOrderRepository } from './InMemoryOrderRepository.js';
import { Order, OrderId, CustomerId, RestaurantId, ItemId, createOrder } from '../../core/domain/Order.js';
import { OrderStatus } from '../../core/domain/OrderStatus.js';
import { Money } from '../../core/domain/Money.js';

describe('InMemoryOrderRepository', () => {
  let repository: InMemoryOrderRepository;

  beforeEach(() => {
    repository = new InMemoryOrderRepository();
  });

  describe('save and findById', () => {
    it('should save and retrieve an order', async () => {
      const order = createOrder({
        customerId: CustomerId('customer-1'),
        restaurantId: RestaurantId('restaurant-1'),
        items: [
          {
            itemId: ItemId('item-1'),
            name: 'Burger',
            quantity: 2,
            unitPrice: Money.of(10, 'USD'),
          },
        ],
      });

      await repository.save(order);

      const retrieved = await repository.findById(order.id);
      expect(retrieved).toEqual(order);
    });

    it('should return null when order is not found', async () => {
      const result = await repository.findById(OrderId('non-existent'));
      expect(result).toBeNull();
    });

    it('should update an existing order when saving with same id', async () => {
      const order = createOrder({
        customerId: CustomerId('customer-1'),
        restaurantId: RestaurantId('restaurant-1'),
        items: [
          {
            itemId: ItemId('item-1'),
            name: 'Pizza',
            quantity: 1,
            unitPrice: Money.of(15, 'USD'),
          },
        ],
      });

      await repository.save(order);

      const updatedOrder: Order = {
        ...order,
        status: OrderStatus.Confirmed,
        updatedAt: new Date(),
      };

      await repository.save(updatedOrder);

      const retrieved = await repository.findById(order.id);
      expect(retrieved).toEqual(updatedOrder);
      expect(retrieved?.status).toBe(OrderStatus.Confirmed);
    });
  });

  describe('findByCustomerId', () => {
    it('should return all orders for a customer', async () => {
      const customerId = CustomerId('customer-1');

      const order1 = createOrder({
        customerId,
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

      const order2 = createOrder({
        customerId,
        restaurantId: RestaurantId('restaurant-2'),
        items: [
          {
            itemId: ItemId('item-2'),
            name: 'Pizza',
            quantity: 1,
            unitPrice: Money.of(15, 'USD'),
          },
        ],
      });

      const order3 = createOrder({
        customerId: CustomerId('customer-2'),
        restaurantId: RestaurantId('restaurant-1'),
        items: [
          {
            itemId: ItemId('item-3'),
            name: 'Salad',
            quantity: 1,
            unitPrice: Money.of(8, 'USD'),
          },
        ],
      });

      await repository.save(order1);
      await repository.save(order2);
      await repository.save(order3);

      const customerOrders = await repository.findByCustomerId(customerId);

      expect(customerOrders).toHaveLength(2);
      expect(customerOrders).toContainEqual(order1);
      expect(customerOrders).toContainEqual(order2);
      expect(customerOrders).not.toContainEqual(order3);
    });

    it('should return empty array when customer has no orders', async () => {
      const result = await repository.findByCustomerId(CustomerId('non-existent'));
      expect(result).toEqual([]);
    });
  });

  describe('updateStatus', () => {
    it('should update order status and updatedAt timestamp', async () => {
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

      await repository.save(order);

      const beforeUpdate = new Date();
      await repository.updateStatus(order.id, OrderStatus.Confirmed);

      const updated = await repository.findById(order.id);

      expect(updated).not.toBeNull();
      expect(updated?.status).toBe(OrderStatus.Confirmed);
      expect(updated?.updatedAt.getTime()).toBeGreaterThanOrEqual(beforeUpdate.getTime());
    });

    it('should preserve all other order properties when updating status', async () => {
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

      await repository.save(order);
      await repository.updateStatus(order.id, OrderStatus.Confirmed);

      const updated = await repository.findById(order.id);

      expect(updated?.id).toBe(order.id);
      expect(updated?.customerId).toBe(order.customerId);
      expect(updated?.restaurantId).toBe(order.restaurantId);
      expect(updated?.items).toEqual(order.items);
      expect(updated?.totalAmount).toEqual(order.totalAmount);
      expect(updated?.createdAt).toEqual(order.createdAt);
    });

    it('should throw error when trying to update non-existent order', async () => {
      await expect(
        repository.updateStatus(OrderId('non-existent'), OrderStatus.Confirmed)
      ).rejects.toThrow('Order with id non-existent not found');
    });

    it('should allow multiple status updates', async () => {
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

      await repository.save(order);

      await repository.updateStatus(order.id, OrderStatus.Confirmed);
      let updated = await repository.findById(order.id);
      expect(updated?.status).toBe(OrderStatus.Confirmed);

      await repository.updateStatus(order.id, OrderStatus.Preparing);
      updated = await repository.findById(order.id);
      expect(updated?.status).toBe(OrderStatus.Preparing);

      await repository.updateStatus(order.id, OrderStatus.OutForDelivery);
      updated = await repository.findById(order.id);
      expect(updated?.status).toBe(OrderStatus.OutForDelivery);
    });
  });
});
