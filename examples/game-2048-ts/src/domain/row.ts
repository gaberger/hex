import type { Cell, Row } from './types.ts';

export interface SlideResult {
  readonly row: Row;
  /** Sum of the merged results. [2,2] gains 4, not 2. */
  readonly gained: number;
}

/**
 * Push four cells to the left. Two equal neighbours become one doubled cell,
 * and that result cannot merge again in the same push.
 *
 * The partner is consumed by stepping the index forward by two. There is no
 * "already merged" flag, so there is no flag to reset wrongly.
 */
export function slideRowLeft(row: Row): SlideResult {
  const squeezed: Cell[] = [];
  for (const cell of row) {
    if (cell !== 0) squeezed.push(cell);
  }

  const merged: Cell[] = [];
  let gained = 0;
  let i = 0;
  while (i < squeezed.length) {
    const current = squeezed[i];
    if (i + 1 < squeezed.length && squeezed[i + 1] === current) {
      const sum = current + current;
      merged.push(sum);
      gained += sum;
      i += 2;
    } else {
      merged.push(current);
      i += 1;
    }
  }
  while (merged.length < 4) merged.push(0);

  return { row: [merged[0], merged[1], merged[2], merged[3]], gained };
}
