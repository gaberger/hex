import { createSignal } from 'solid-js';
import { useNavigate } from '@reach/router';
import { useAuth } from '../state/auth';

const Login = () => {
  const [email, setEmail] = createSignal('');
  const [password, setPassword] = createSignal('');
  const [error, setError] = createSignal('');
  const navigate = useNavigate();
  const [, { setToken }] = useAuth();

  const handleSubmit = async (event: Event) => {
    event.preventDefault();
    setError('');

    try {
      // Simulate an API call to login
      const response = await fetch('/api/login', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ email: email(), password: password() }),
      });

      if (!response.ok) {
        throw new Error('Invalid email or password');
      }

      const data = await response.json();
      setToken(data.jwt);
      sessionStorage.setItem('jwt', data.jwt);

      navigate('/');
    } catch (error) {
      setError((error as Error).message);
    }
  };

  return (
    <div>
      <h2>Login</h2>
      <form onSubmit={handleSubmit}>
        <div>
          <label for="email">Email:</label>
          <input
            id="email"
            type="email"
            value={email()}
            onInput={(e) => setEmail(e.currentTarget.value)}
            required
          />
        </div>
        <div>
          <label for="password">Password:</label>
          <input
            id="password"
            type="password"
            value={password()}
            onInput={(e) => setPassword(e.currentTarget.value)}
            required
          />
        </div>
        {error() && <p style={{ color: 'red' }}>{error()}</p>}
        <button type="submit">Login</button>
      </form>
    </div>
  );
};

export default Login;
docs/specs/ebay-spec-004