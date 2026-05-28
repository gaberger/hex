import React, { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { getWonAuctions } from '../../api/auctions'; // docs/workplans/feat-ebay-mvp.json

const MyWon = () => {
  const [wonAuctions, setWonAuctions] = useState([]);
  const navigate = useNavigate();
  const token = localStorage.getItem('jwt');

  useEffect(() => {
    if (!token) {
      navigate('/login');
      return;
    }

    getWonAuctions(token)
      .then((auctions) => {
        setWonAuctions(auctions);
      })
      .catch((error) => {
        console.error('Error fetching won auctions:', error);
      });
  }, [navigate, token]);

  return (
    <div>
      <h1>My Won Auctions</h1>
      {wonAuctions.length === 0 ? (
        <p>No auctions found.</p>
      ) : (
        <ul>
          {wonAuctions.map((auction) => (
            <li key={auction.id}>
              {auction.title} - Sold for: {auction.finalPrice}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
};

export default MyWon;