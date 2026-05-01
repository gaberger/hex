export enum OrderStatus {
  Pending = 'PENDING',
  Confirmed = 'CONFIRMED',
  Preparing = 'PREPARING',
  ReadyForPickup = 'READY_FOR_PICKUP',
  InTransit = 'IN_TRANSIT',
  Delivered = 'DELIVERED',
  Cancelled = 'CANCELLED',
}

const validTransitions: Record<OrderStatus, OrderStatus[]> = {
  [OrderStatus.Pending]: [OrderStatus.Confirmed, OrderStatus.Cancelled],
  [OrderStatus.Confirmed]: [OrderStatus.Preparing, OrderStatus.Cancelled],
  [OrderStatus.Preparing]: [OrderStatus.ReadyForPickup, OrderStatus.Cancelled],
  [OrderStatus.ReadyForPickup]: [OrderStatus.InTransit, OrderStatus.Cancelled],
  [OrderStatus.InTransit]: [OrderStatus.Delivered],
  [OrderStatus.Delivered]: [],
  [OrderStatus.Cancelled]: [],
};

export function isValidStatusTransition(
  from: OrderStatus,
  to: OrderStatus
): boolean {
  return validTransitions[from].includes(to);
}
