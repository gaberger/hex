import { Route, Router } from 'solid-app-router';
import PostListing from './pages/PostListing';
import Register from './pages/Register';
import Login from './pages/Login';
import Home from './pages/Home'; // Added import for Home page
import MyBids from './pages/MyBids'; // Import for MyBids page
import MyWon from './pages/MyWon'; // Import for MyWon page
import MyListings from './pages/MyListings'; // Import for MyListings page
import ListingDetail from './pages/ListingDetail'; // Added import for ListingDetail page

function App() {
  return (
    <Router>
      <Route path="/" element={<Home />} /> {/* Updated to use Home component */}
      <Route path="/listings" element={<div>Listings Page</div>} />
      <Route path="/post-listing" element={<PostListing />} /> {/* Added route for post-listing page */}
      <Route path="/register" element={<Register />} />
      <Route path="/login" element={<Login />} />
      <Route path="/my-bids" element={<MyBids />} /> {/* Route for MyBids page */}
      <Route path="/my-won" element={<MyWon />} /> {/* Route for MyWon page */}
      <Route path="/my-listings" element={<MyListings />} /> {/* Route for MyListings page */}
      <Route path="/listing/:id" element={<ListingDetail />} /> {/* Added route for ListingDetail page */}
      {/* Placeholder for more routes */}
    </Router>
  );
}

export default App; // ADR-2026-05-19-0721