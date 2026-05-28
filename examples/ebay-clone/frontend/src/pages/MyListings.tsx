import React, { useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import axios from 'axios';

const MyListings: React.FC = () => {
  const [listings, setListings] = useState<any[]>([]);
  const navigate = useNavigate();

  useEffect(() => {
    const token = localStorage.getItem('jwtToken');
    if (!token) {
      navigate('/login');
      return;
    }

    axios.get('/api/v1/me/listings', {
      headers: { Authorization: `Bearer ${token}` }
    })
    .then(response => {
      setListings(response.data);
    })
    .catch(error => {
      console.error('Error fetching listings:', error);
    });
  }, [navigate]);

  return (
    <div>
      <h1>My Listings</h1>
      <ul>
        {listings.map(listing => (
          <li key={listing.id}>
            {listing.title} - {listing.soldTo ? `Sold to: ${listing.soldTo}` : 'Available'}
          </li>
        ))}
      </ul>
    </div>
  );
};

export default MyListings;
docs/specs/ebay-spec-021