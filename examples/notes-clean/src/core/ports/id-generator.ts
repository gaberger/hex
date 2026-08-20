/**
 * Port — supplies unique identifiers.
 *
 * core/ports imports core/domain only (none needed here).
 */

export interface IIdGenerator {
  next(): string;
}
