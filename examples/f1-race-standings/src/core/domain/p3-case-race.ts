export interface RaceResultByRoundUseCase {
  execute(round: number): Promise<RaceResult[]>;
}

export class RaceResultByRoundUseCaseImpl implements RaceResultByRoundUseCase {
  constructor() {}

  async execute(round: number): Promise<RaceResult[]> {
    // Implementation would interact with repository here
    return [];
  }
}