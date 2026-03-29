import {ErgastPort } from '@ports/primary/ergast/ergast-port'

class CliAdapter implements ErgastPort {
  constructor() {}

  async fetchRaces(): Promise<void> {
    // Implementation would go here
  }

  async fetchCircuit(circuitId: string): Promise<void> {
    // Implementation would go here
  }
}