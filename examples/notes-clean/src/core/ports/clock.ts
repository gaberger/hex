/**
 * Port — supplies the current time as an epoch millisecond timestamp.
 */

export interface IClock {
  now(): number;
}
