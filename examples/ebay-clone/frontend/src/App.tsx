import { Route, Router } from 'solid-app-router';

function App() {
  return (
    <Router>
      <Route path="/" element={<div>Home Page</div>} />
      <Route path="/listings" element={<div>Listings Page</div>} />
      {/* Placeholder for more routes */}
    </Router>
  );
}

export default App; // ADR-2026-05-19-0721