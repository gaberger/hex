// src/core/domain/P0.1.ts

export interface Security {
  id: string;
  name: string;
  type: string;
}

export interface RiskMetrics {
  valueAtRisk: number;
  expectedShortfall: number;
}

export interface Position {
  security: Security;
  quantity: number;
  riskMetrics: RiskMetrics;
}

export class Portfolio {
  private positions: Position[];

  constructor(positions: Position[] = []) {
    this.positions = positions;
  }

  public addPosition(position: Position): void {
    this.positions.push(position);
  }

  public getPositions(): Position[] {
    return this.positions;
  }

  public calculateTotalValueAtRisk(): number {
    return this.positions.reduce((total, position) => total + position.riskMetrics.valueAtRisk, 0);
  }

  public calculateTotalExpectedShortfall(): number {
    return this.positions.reduce((total, position) => total + position.riskMetrics.expectedShortfall, 0);
  }
}