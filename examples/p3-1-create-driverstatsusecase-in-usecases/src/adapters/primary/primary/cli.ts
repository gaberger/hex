import{ DriverStatsUseCase} from '@ports/driver-stats-usecase';

export class CliDriverStatsPresenter {
  constructor(private readonly useCase: DriverStatsUseCase) {}
}