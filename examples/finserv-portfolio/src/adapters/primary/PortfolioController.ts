import { Request, Response } from 'express';
import { CreatePortfolioUseCase, DeletePortfolioUseCase, GetPortfolioUseCase, UpdatePortfolioUseCase } from '../../usecases/PortfolioUseCases.js';
import { Portfolio } from '../../domain/Portfolio.js';

export class PortfolioController {
  private createPortfolioUseCase: CreatePortfolioUseCase;
  private getPortfolioUseCase: GetPortfolioUseCase;
  private updatePortfolioUseCase: UpdatePortfolioUseCase;
  private deletePortfolioUseCase: DeletePortfolioUseCase;

  constructor(
    createPortfolioUseCase: CreatePortfolioUseCase,
    getPortfolioUseCase: GetPortfolioUseCase,
    updatePortfolioUseCase: UpdatePortfolioUseCase,
    deletePortfolioUseCase: DeletePortfolioUseCase
  ) {
    this.createPortfolioUseCase = createPortfolioUseCase;
    this.getPortfolioUseCase = getPortfolioUseCase;
    this.updatePortfolioUseCase = updatePortfolioUseCase;
    this.deletePortfolioUseCase = deletePortfolioUseCase;
  }

  async create(req: Request, res: Response): Promise<void> {
    try {
      const { name, description } = req.body;
      const portfolio: Portfolio = await this.createPortfolioUseCase.execute(name, description);
      res.status(201).json(portfolio);
    } catch (error) {
      res.status(400).json({ message: error.message });
    }
  }

  async get(req: Request, res: Response): Promise<void> {
    try {
      const { id } = req.params;
      const portfolio: Portfolio | null = await this.getPortfolioUseCase.execute(id);
      if (!portfolio) {
        res.status(404).json({ message: 'Portfolio not found' });
        return;
      }
      res.json(portfolio);
    } catch (error) {
      res.status(400).json({ message: error.message });
    }
  }

  async update(req: Request, res: Response): Promise<void> {
    try {
      const { id } = req.params;
      const { name, description } = req.body;
      const portfolio: Portfolio = await this.updatePortfolioUseCase.execute(id, name, description);
      res.json(portfolio);
    } catch (error) {
      res.status(400).json({ message: error.message });
    }
  }

  async delete(req: Request, res: Response): Promise<void> {
    try {
      const { id } = req.params;
      await this.deletePortfolioUseCase.execute(id);
      res.status(204).json();
    } catch (error) {
      res.status(400).json({ message: error.message });
    }
  }
}