import React, { useState, useEffect } from 'react';
import { useNavigate, useLocation } from 'react-router-dom';
import queryString from 'query-string';
import axios from 'axios';
import ListingCard from '../components/ListingCard';
import SearchBar from '../components/SearchBar';

const Home = () => {
  const [listings, setListings] = useState([]);
  const [searchTerm, setSearchTerm] = useState('');
  const [maxPrice, setMaxPrice] = useState(1000);
  const [sortByEndTime, setSortByEndTime] = useState(true);
  const [currentPage, setCurrentPage] = useState(1);
  const navigate = useNavigate();
  const location = useLocation();

  useEffect(() => {
    const params = queryString.parse(location.search);
    setSearchTerm(params.q || '');
    setMaxPrice(Number(params.maxPrice) || 1000);
    setSortByEndTime(params.sort === 'end_time');
    setCurrentPage(Number(params.page) || 1);
  }, [location]);

  useEffect(() => {
    const fetchListings = async () => {
      try {
        const response = await axios.get('/api/v1/listings', {
          params: {
            q: searchTerm,
            maxPrice: maxPrice,
            sortByEndTime: sortByEndTime ? 'asc' : undefined,
            page: currentPage,
          },
        });
        setListings(response.data.listings.filter(listing => listing.active));
      } catch (error) {
        console.error('Error fetching listings:', error);
      }
    };

    fetchListings();
  }, [searchTerm, maxPrice, sortByEndTime, currentPage]);

  useEffect(() => {
    const queryParams = queryString.stringify({
      q: searchTerm,
      maxPrice: maxPrice,
      sort: sortByEndTime ? 'end_time' : undefined,
      page: currentPage,
    });
    navigate(`?${queryParams}`);
  }, [searchTerm, maxPrice, sortByEndTime, currentPage]);

  return (
    <div>
      <h1>Active Auctions</h1>
      <SearchBar
        searchTerm={searchTerm}
        setSearchTerm={setSearchTerm}
        debounceTimeout={250}
      />
      <input
        type="range"
        min="0"
        max="1000"
        value={maxPrice}
        onChange={(e) => setMaxPrice(Number(e.target.value))}
      />
      <label>Max Price: ${maxPrice}</label>
      <button onClick={() => setSortByEndTime(!sortByEndTime)}>
        Sort by {sortByEndTime ? 'End Time' : 'Default'}
      </button>
      <div className="listing-container">
        {listings.map((listing) => (
          <ListingCard key={listing.id} listing={listing} />
        ))}
      </div>
      <div className="pagination">
        <button onClick={() => setCurrentPage(currentPage - 1)} disabled={currentPage === 1}>
          Previous
        </button>
        <span>Page {currentPage}</span>
        <button onClick={() => setCurrentPage(currentPage + 1)}>
          Next
        </button>
      </div>
    </div>
  );
};

export default Home;
docs/workplans/feat-ebay-mvp.json