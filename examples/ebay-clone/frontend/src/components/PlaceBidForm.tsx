import React, { useState, useEffect } from 'react';
import { useDispatch, useSelector } from 'react-redux';
import { placeBid } from '../../redux/actions/auctionActions';
import { toast } from 'react-toastify';

const PlaceBidForm = ({ auctionId, currentHighestBid }) => {
  const dispatch = useDispatch();
  const [bidAmount, setBidAmount] = useState('');
  const auctionStatus = useSelector(state => state.auctions[auctionId]?.status);
  const endTime = useSelector(state => state.auctions[auctionId]?.end_time);

  useEffect(() => {
    if (!auctionStatus || !endTime) return;
    const timer = setInterval(() => {
      const now = new Date();
      const end = new Date(endTime);
      if (now >= end) clearInterval(timer);
    }, 1000);

    return () => clearInterval(timer);
  }, [auctionStatus, endTime]);

  const handleSubmit = (e) => {
    e.preventDefault();

    if (auctionStatus !== 'Active') {
      toast.error('Auction is not active');
      return;
    }

    const now = new Date();
    const end = new Date(endTime);
    if (now >= end) {
      toast.error('Auction has ended');
      return;
    }

    if (parseFloat(bidAmount) <= currentHighestBid) {
      toast.error(`Bid must be higher than the current highest bid: ${currentHighestBid}`);
      return;
    }

    dispatch(placeBid(auctionId, bidAmount))
      .then(() => {
        setBidAmount('');
        toast.success('Your bid has been placed successfully!');
      })
      .catch(error => {
        if (error.response.status === 409) {
          toast.error('Someone else placed a higher bid. Please try again.');
        } else if (error.response.status === 410) {
          toast.error('Auction has ended.');
        } else if (error.response.status === 403) {
          toast.error('You are not authorized to place a bid.');
        }
      });
  };

  return (
    <form onSubmit={handleSubmit}>
      <div>
        <label htmlFor="bidAmount">Your Bid:</label>
        <input
          type="number"
          id="bidAmount"
          value={bidAmount}
          onChange={(e) => setBidAmount(e.target.value)}
          disabled={auctionStatus !== 'Active'}
        />
      </div>
      <button type="submit" disabled={auctionStatus !== 'Active'}>
        Place Bid
      </button>
    </form>
  );
};

export default PlaceBidForm;

// ADR-2026-05-19-0721