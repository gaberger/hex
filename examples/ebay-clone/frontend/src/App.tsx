import { Route, Router } from 'solid-app-router';
import PostListing from './pages/PostListing';
import Register from './pages/Register';
import Login from './pages/Login';
import Home from './pages/Home'; // Added import for Home page

function App() {
  return (
    <Router>
      <Route path="/" element={<Home />} /> {/* Updated to use Home component */}
      <Route path="/listings" element={<div>Listings Page</div>} />
      <Route path="/post-listing" element={<PostListing />} /> {/* Added route for post-listing page */}
      <Route path="/register" element={<Register />} />
      <Route path="/login" element={<Login />} />
      {/* Placeholder for more routes */}
    </Router>
  );
}

export default App; // ADR-2026-05-19-0721