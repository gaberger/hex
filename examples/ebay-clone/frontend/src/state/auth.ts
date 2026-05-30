import { createSignal } from 'solid-js';

export const useAuth = () => {
  const [token, setToken] = createSignal<string | null>(sessionStorage.getItem('auth_token') || null);

  const login = ( (username: string, password: string) => {
    try {
      const res = await fetch('/api/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ username, password })
      });

      if (!res.ok) throw new Error('Login failed');

      const { token } = await res.json();
      setToken(token);
      sessionStorage.setItem('auth_token', token);
      return true;
    } catch (err) {
      return false;
    }
  };

  const logout = () => {
    setToken(null);
    sessionStorage.removeItem('auth_token');
  };

  return {
    token,
    login,
    logout
  };
};