export interface Money {
  readonly amount: number;
  readonly currency: string;
}

export function createMoney(amount: number, currency: string = 'USD'): Money {
  if (amount < 0) {
    throw new Error('Money amount cannot be negative');
  }
  return { amount, currency };
}

export function addMoney(a: Money, b: Money): Money {
  if (a.currency !== b.currency) {
    throw new Error(`Cannot add money with different currencies: ${a.currency} and ${b.currency}`);
  }
  return createMoney(a.amount + b.amount, a.currency);
}
