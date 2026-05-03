import type { ICreateOrderUseCase, CreateOrderInput } from '../../core/ports/ICreateOrderUseCase.js';
import type { IUpdateOrderStatusUseCase } from '../../core/ports/IUpdateOrderStatusUseCase.js';
import { OrderStatus } from '../../core/domain/OrderStatus.js';

export class HttpServer {
  constructor(
    private readonly createOrder: ICreateOrderUseCase,
    private readonly updateStatus: IUpdateOrderStatusUseCase,
  ) {}

  async handle(req: { path: string; body: unknown }): Promise<unknown> {
    if (req.path === '/orders' && typeof req.body === 'object' && req.body) {
      return this.createOrder.execute(req.body as CreateOrderInput);
    }
    if (req.path.startsWith('/orders/') && typeof req.body === 'object' && req.body) {
      const id = req.path.slice('/orders/'.length);
      const { status } = req.body as { status: OrderStatus };
      await this.updateStatus.execute(id, status);
      return { ok: true };
    }
    throw new Error(`unknown route: ${req.path}`);
  }
}
