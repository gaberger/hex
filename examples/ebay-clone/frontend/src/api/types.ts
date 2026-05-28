// ADR-2026-05-19-0721

import { bigint } from "prop-types";

export interface Listing {
  id: string;
  title: string;
  description: string;
  startingPrice: bigint | number; // in cents, never a float
  currentPrice: bigint | number; // in cents, never a float
  sellerId: string;
  createdAt: Date;
  updatedAt: Date;
}

export interface Auction {
  id: string;
  listingId: string;
  startTime: Date;
  endTime: Date;
  currentHighestBid: Bid | null;
  bids: Bid[];
}

export interface Bid {
  id: string;
  auctionId: string;
  bidderId: string;
  amount: bigint | number; // in cents, never a float
  timestamp: Date;
}

export interface User {
  id: string;
  username: string;
  email: string;
  createdAt: Date;
  updatedAt: Date;
}
// docs/specs/ebay-spec-020
// docs/specs/ebay-spec-025