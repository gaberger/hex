export class Driver {
  constructor(
public readonly id: string,
    public readonly name: string,
    public readonly team: string,
  ) {}
}

export class Race {
  constructor(
    public readonly id: string,
    public readonly date: Date,
    public readonly circuit: string,
    public readonly positions: Position[],
  ) {}
}

export class Season {
  constructor(
    public readonly id: string,
    public readonly year: number,
    public readonly races: string[],
  ) {}
}

export class Constructor {
  constructor(
    public readonly id: string,
    public readonly name: string,
    public readonly drivers: string[],
  ) {}
}

export class Position {
  constructor(
    public readonly rank: number,
    public readonly points: Points,
  ) {}
}

export class Points {
  constructor(
    public readonly value: number,
  ) {}
}

export class LapTime {
  constructor(
    public readonly time: number,
  ) {}
}