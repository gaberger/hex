import React from 'react';
import { Listing } from 'docs/specs/ebay-spec-019';

interface ListingCardProps {
  listing: Listing;
}

const ListingCard: React.FC<ListingCardProps> = ({ listing }) => {
  const { title, coverImage, currentPrice, endTime } = listing;

  // Calculate time remaining
  const now = new Date();
  const endDateTime = new Date(endTime);
  const timeRemaining = Math.ceil((endDateTime.getTime() - now.getTime()) / (1000 * 60));

  return (
    <div className="listing-card">
      <img src={coverImage} alt={`${title} cover`} />
      <h2>{title}</h2>
      <p>Current Price: ${currentPrice.toFixed(2)}</p>
      <p>Time Remaining: {timeRemaining} minutes</p>
    </div>
  );
};

export default ListingCard;