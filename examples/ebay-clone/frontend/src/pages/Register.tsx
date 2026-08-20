import { createSignal, Show } from 'solid-js';
import { useNavigate } from '@solidjs/router';
import { http } from '../api/http';
import { API_BASE, setToken } from '../api/auth';

export default function Register() {
  const [username, setUsername] = createSignal('');
  const [password, setPassword] = createSignal('');
  const [error, setError] = createSignal('');
  const navigate = useNavigate();

  async function submit(e: Event) {
    e.preventDefault();
    setError('');
    try {
      const r = await http<{ token: string }>('POST', `${API_BASE}/api/v1/auth/register`, {
        username: username(),
        email: '',
        password: password(),
      });
      setToken(r.token);
      navigate('/');
    } catch (err: any) {
      setError(String(err?.message ?? err));
    }
  }

  return (
    <form onSubmit={submit} class="p-6 max-w-sm mx-auto space-y-3">
      <h1 class="text-2xl font-bold">Register</h1>
      <input class="border p-2 w-full rounded" placeholder="username"
        value={username()} onInput={(e) => setUsername(e.currentTarget.value)} />
      <input class="border p-2 w-full rounded" type="password" placeholder="password (min 6)"
        value={password()} onInput={(e) => setPassword(e.currentTarget.value)} />
      <button class="bg-blue-600 text-white px-4 py-2 rounded" type="submit">Create account</button>
      <Show when={error()}><p class="text-red-600">{error()}</p></Show>
    </form>
  );
}
