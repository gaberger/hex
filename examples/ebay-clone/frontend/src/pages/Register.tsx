import { createSignal } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { useAuth } from '../state/auth';

const Register = () => {
  const navigate = useNavigate();
  const [email, setEmail] = createSignal('');
  const [password, setPassword] = createSignal('');
  const [error, setError] = createSignal('');
  const { setToken } = useAuth();

  const handleSubmit = async (event: Event) => {
    event.preventDefault();
    try {
      // Mock API call for demonstration purposes
      const response = await fetch('https://api.example.com/register', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ email: email(), password: password() }),
      });

      if (!response.ok) {
        throw new Error('Registration failed');
      }

      const data = await response.json();
      setToken(data.jwt);
      sessionStorage.setItem('jwt', data.jwt);

      // Redirect to home page on success
      navigate('/');
    } catch (err) {
      setError('Invalid email or password');
    }
  };

  return (
    <div>
      <h1>Register</h1>
      <form onSubmit={handleSubmit}>
        <input type="email" value={email()} onInput={(e) => setEmail(e.currentTarget.value)} placeholder="Email" required />
        <br />
        <input
          type="password"
          value={password()}
          onInput={(e) => setPassword(e.currentTarget.value)}
          placeholder="Password"
          required
        />
        <br />
        {error() && <p style={{ color: 'red' }}>{error()}</p>}
        <button type="submit">Register</button>
      </form>
    </div>
  );
};

export default Register;
// docs/specs/ebay-spec-001