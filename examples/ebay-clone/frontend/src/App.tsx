import { Router, Routes } from '@solidjs/router';
import Home from './pages/Home';
import Register from './pages/Register';
import Login from './pages/Login';

export default function App() {
  return (
    <Router>
      <Routes>
        <Route path="" component={Home} />
        <Route path="/register" component={Register} />
        <Route path="/login" component={Login} />
      </Routes>
    </Router>
  );
}