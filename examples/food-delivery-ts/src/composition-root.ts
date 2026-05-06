import { InMemoryOrderRepository } from './adapters/secondary/InMemoryOrderRepository.js';
import { HttpServer } from './adapters/primary/HttpServer.js';
import { CreateOrderUseCase } from './core/usecases/CreateOrder.js';
import { UpdateOrderStatusUseCase } from './core/usecases/UpdateOrderStatus.js';

export function compose() {
  const orderRepo = new InMemoryOrderRepository();
  const createOrder = new CreateOrderUseCase(orderRepo);
  const updateStatus = new UpdateOrderStatusUseCase(orderRepo);
  const server = new HttpServer(createOrder, updateStatus);
  return { server, orderRepo };
}
