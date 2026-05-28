import type { Listing, Auction, Bid, User } from './types';

// ADR-2026-05-19-0721: Ensuring all monetary values are handled with high precision and never use floats

/**
 * Formats cents (as bigint or number) into a USD string.
 * @param cents - The amount in cents to format.
 * @returns Formatted USD string, e.g., "$1,234.56".
 */
export function formatUSD(cents: bigint | number): string {
  const value = typeof cents === 'bigint' ? Number(cents) : cents;
  return new Intl.NumberFormat('en-US', {
    style: 'currency',
    currency: 'USD',
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(value / 100);
}