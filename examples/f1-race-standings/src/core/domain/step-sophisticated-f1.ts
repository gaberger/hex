export interface Driver {
  id: string;
  name: string;
  team: string;
  position: Position;
  points: Points;
  raceResults: RaceResult[];
}

export interface RaceResult {
  raceId: string;
  position: Position;
  points: Points;
  laps: number;
  time: string;
  fastestLap: string | null;
}

export interface Constructor {
  id: string;
  name: string;
  drivers: Driver[];
  points: Points;
}

export interface Position {
  value: number;
  isLeading: boolean;
}

export interface Points {
  value: number;
  isBonus: boolean;
}

export interface SeasonData {
  year: number;
  races: Race[];
  standings: DriverStandings;
}

export interface Race {
  id: string;
  round: number;
  date: string;
  circuit: string;
  results: RaceResult[];
}

export interface DriverStandings {
  drivers: Driver[];
  constructors: Constructor[];
}

export interface HistoricalData {
  seasons: SeasonData[];
}