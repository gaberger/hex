import React, { useEffect, useState } from 'react';
import { useSubscription } from '@apollo/client';
import { gql } from '@apollo/client';

const BIDS_SUBSCRIPTION = gql`
  subscription GetBids($auctionId: ID!) {
    bid(where: { auction_id: { _eq: $auctionId } }) {
      amount
      created_at
      bidder {
        username
      }
    }
  }
`;

const BidFeed = ({ auctionId, highestBid }) => {
  const [bids, setBids] = useState([]);
  const { data, error, loading } = useSubscription(BIDS_SUBSCRIPTION, {
    variables: { auctionId },
  });

  useEffect(() => {
    if (!loading && !error) {
      setBids((prevBids) => [data.bid, ...prevBids]);
    }
  }, [data, error, loading]);

  return (
    <div className="bid-feed">
      <h3>Bid History</h3>
      {bids.length > 0 ? (
        bids.map((bid, index) => (
          <div key={index} className="bid-item">
            <span>{bid.bidder.username}</span> bid <strong>${bid.amount}</strong> at {new Date(bid.created_at).toLocaleString()}
          </div>
        ))
      ) : (
        <p>No bids yet</p>
      )}
    </div>
  );
};

export default BidFeed;
docs/specs/ebay-spec-020