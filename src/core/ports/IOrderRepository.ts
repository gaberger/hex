import type { Order, OrderId, CustomerId } from '../domain/Order.js';
import type { OrderStatus } from '../domain/OrderStatus.js';

/**
 * Port interface for order persistence operations.
 * Secondary adapters implement this interface to provide data access.
 */
export interface IOrderRepository {
  /**
   * Find an order by its unique identifier
   * @param orderId - The order identifier
   * @returns The order if found, null otherwise
   */
  findById(orderId: OrderId): Promise<Order | null>;

  /**
   * Persist an order (create or update)
   * @param order - The order to save
   */
  save(order: Order): Promise<void>;

  /**
   * Find all orders for a specific customer
   * @param customerId - The customer identifier
   * @returns Array of orders (empty if none found)
   */
  findByCustomerId(customerId: CustomerId): Promise<Order[]>;

  /**
   * Update the status of an existing order
   * @param orderId - The order identifier
   * @param status - The new status
   * @throws Error if order not found or status transition invalid
   */
  updateStatus(orderId: OrderId, status: OrderStatus): Promise<void>;
}
