import { Route, Router } from '@solidjs/router';
import PostListing from './pages/PostListing';
import Register from './pages/Register';
import Login from './pages/Login';
import Home from './pages/Home';
import MyBids from './pages/MyBids';
import MyWon from './pages/MyWon';
import MyListings from './pages/MyListings';

// @solidjs/router v0.14 API: routes declared with `component`, nested directly
// under <Router> (no <Routes> wrapper). Replaces the renamed/removed
// `solid-app-router` package the scaffold was written against.
function App() {
  return (
    <Router>
      <Route path="/" component={Home} />
      <Route path="/listings" component={() => <div>Listings Page</div>} />
      <Route path="/post-listing" component={PostListing} />
      <Route path="/register" component={Register} />
      <Route path="/login" component={Login} />
      <Route path="/my-bids" component={MyBids} />
      <Route path="/my-won" component={MyWon} />
      <Route path="/my-listings" component={MyListings} />
    </Router>
  );
}

export default App; // ADR-2026-05-19-0721
