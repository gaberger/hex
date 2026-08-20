import { createSignal } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { setToken } from '../api/auth';

// The in-memory backend's bearer token == username (UsernameTokenIssuer), so
// "login" stores the username as the token. A password field is shown for
// parity but the demo backend does not verify it (no User.password_hash in the
// domain model — see project notes).
export default function Login() {
  const [username, setUsername] = createSignal('');
  const navigate = useNavigate();

  function submit(e: Event) {
    e.preventDefault();
    if (!username()) return;
    setToken(username());
    navigate('/');
  }

  return (
    <form onSubmit={submit} class="p-6 max-w-sm mx-auto space-y-3">
      <h1 class="text-2xl font-bold">Login</h1>
      <input class="border p-2 w-full rounded" placeholder="username"
        value={username()} onInput={(e) => setUsername(e.currentTarget.value)} />
      <button class="bg-blue-600 text-white px-4 py-2 rounded" type="submit">Continue</button>
    </form>
  );
}
