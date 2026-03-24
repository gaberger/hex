import { type PortfolioRepository } from '../ports/secondary/PortfolioRepository.js';
import { type GetPortfolioQuery } from '../ports/primary/GetPortfolioQuery.js';
import { type CreatePortfolioCommand } from '../ports/primary/CreatePortfolioCommand.js';
import { type UpdatePortfolioCommand } from '../ports/primary/UpdatePortfolioCommand.js';
import { Portfolio } from '../domain/Portfolio.js';
import { type PortfolioId } from '../domain/PortfolioId.js';

export class PortfolioUseCases {
  private readonly portfolioRepository: PortfolioRepository;

  constructor(portfolioRepository: PortfolioRepository) {
    this.portfolioRepository = portfolioRepository;
  }

  async createPortfolio(command: CreatePortfolioCommand): Promise<PortfolioId> {
    const portfolio = Portfolio.create(command.name, command.ownerId);
    await this.portfolioRepository.save(portfolio);
    return portfolio.id;
  }

  async getPortfolio(query: GetPortfolioQuery): Promise<Portfolio> {
    return this.portfolioRepository.load(query.portfolioId);
  }

  async updatePortfolio(command: UpdatePortfolioCommand): Promise<void> {
    const portfolio = await this.portfolioRepository.load(command.portfolioId);
    portfolio.update(command.name);
    await this.portfolioRepository.save(portfolio);
  }
}