import React, { useEffect, useState } from 'react';
import Countdown from '../components/Countdown';
import BidFeed from '../components/BidFeed';
import PlaceBidForm from '../components/PlaceBidForm';
import { useParams } from 'react-router-dom';
import { Auction, Bid } from '../../types';
import { fetchAuctionById, fetchBidsByAuctionId, placeBid } from '../../api';

const ListingDetail: React.FC = () => {
  const { id } = useParams<{ id: string }>();
  const [auction, setAuction] = useState<Auction | null>(null);
  const [bids, setBids] = useState<Bid[]>([]);
  const [currentHighestBid, setCurrentHighestBid] = useState<number>(0);
  const [formDisabled, setFormDisabled] = useState<boolean>(false);

  useEffect(() => {
    const fetchInitialData = async () => {
      try {
        const auctionData: Auction = await fetchAuctionById(id);
        setAuction(auctionData);
        setCurrentHighestBid(auctionData.current_highest_bid || 0);

        if (auctionData.status !== 'Active' || new Date(auctionData.end_time) <= new Date()) {
          setFormDisabled(true);
        }

        const bidsData: Bid[] = await fetchBidsByAuctionId(id);
        setBids(bidsData);
      } catch (error) {
        console.error('Error fetching auction or bid data:', error);
      }
    };

    fetchInitialData();
  }, [id]);

  useEffect(() => {
    const interval = setInterval(async () => {
      if (!auction || new Date(auction.end_time) <= new Date()) {
        setFormDisabled(true);
        clearInterval(interval);
        return;
      }

      try {
        const updatedAuction: Auction = await fetchAuctionById(id);
        setAuction(updatedAuction);
        setCurrentHighestBid(updatedAuction.current_highest_bid || 0);

        const newBids: Bid[] = await fetchBidsByAuctionId(id);
        if (newBids.length > bids.length) {
          setBids([newBids[newBids.length - 1], ...bids]);
        }
      } catch (error) {
        console.error('Error updating auction or bid data:', error);
      }
    }, 1000);

    return () => clearInterval(interval);
  }, [auction, bids, id]);

  const handleBidPlacement = async (bidAmount: number) => {
    if (!auction || formDisabled) {
      return;
    }

    if (bidAmount <= currentHighestBid) {
      alert('Bid amount must be higher than the current highest bid.');
      return;
    }

    try {
      await placeBid(id, bidAmount);
      const updatedAuction: Auction = await fetchAuctionById(id);
      setAuction(updatedAuction);
      setCurrentHighestBid(updatedAuction.current_highest_bid || 0);

      const newBids: Bid[] = await fetchBidsByAuctionId(id);
      setBids([newBids[newBids.length - 1], ...bids]);
    } catch (error) {
      if (error.response && [409, 410, 403].includes(error.response.status)) {
        alert(`Error placing bid: ${error.response.statusText}`);
      }
    }
  };

  return (
    <div>
      {auction ? (
        <>
          <h1>{auction.title}</h1>
          <p>{auction.description}</p>
          <Countdown endTime={new Date(auction.end_time)} />
          <p>Current Highest Bid: ${currentHighestBid}</p>
          <BidFeed bids={bids} />
          <PlaceBidForm onSubmit={handleBidPlacement} disabled={formDisabled} currentHighestBid={currentHighestBid} />
        </>
      ) : (
        <p>Loading...</p>
      )}
    </div>
  );
};

export default ListingDetail;

docs/specs/ebay-spec-020