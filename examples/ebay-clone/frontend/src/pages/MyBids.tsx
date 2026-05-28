import React, { useEffect, useState } from 'react';
import axios from 'axios';
import { useNavigate } from 'react-router-dom';

const MyBids = () => {
  const [bids, setBids] = useState([]);
  const navigate = useNavigate();
  const token = localStorage.getItem('jwtToken');

  useEffect(() => {
    if (!token) {
      navigate('/login');
      return;
    }

    axios.get('/api/v1/me/bids', { headers: { Authorization: `Bearer ${token}` } })
      .then(response => setBids(response.data))
      .catch(error => console.error('Error fetching bids:', error));
  }, [navigate, token]);

  if (!token) return null;

  return (
    <div>
      <h1>My Bids</h1>
      {bids.length === 0 ? (
        <p>No bids found.</p>
      ) : (
        <ul>
          {bids.map(bid => (
            <li key={bid.id}>
              <strong>{bid.auction_title}</strong> - Bid Amount: {bid.amount}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};

export default MyBids;

// docs/workplans/feat-ebay-mvp.json