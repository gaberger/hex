export class SQLiteAdapter implements ISecondaryAdapter {
  constructor(private readonly db: IDatabase) {}

  async saveRaceResults(raceResults: RaceResults): Promise<void> {
    await this.db.saveRaceResults(raceResults);
  }

  async getRaceResults(raceId: string): Promise<RaceResults> {
    return await this.db.getRaceResults(raceId);
  }
}

export class RabbitMQAdapter implements ISecondaryAdapter {
  constructor(private readonly queue: IQueue) {}

  async publishRaceResults(raceResults: RaceResults): Promise<void> {
    await this.queue.publishRaceResults(raceResults);
  }
}