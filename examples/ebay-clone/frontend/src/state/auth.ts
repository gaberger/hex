import { createSignal } from 'solid-js';
import { navigate } from '@solidjs/router';

// ADR-2026-05-19-0721: Stores JWT in memory + sessionStorage

const [token, setToken] = createSignal<string | null>(sessionStorage.getItem('jwt-token') || null);

export const useAuth = () => {
  const login = async (email: string, password: string) => {
    try {
      // Mock API call
      const response = await fetch('/api/login', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ email, password }),
      });

      if (response.ok) {
        const data = await response.json();
        setToken(data.token);
        sessionStorage.setItem('jwt-token', data.token);
        navigate('/');
      } else {
        throw new Error('Invalid credentials');
      }
    } catch (error) {
      console.error('Login failed:', error);
      return false;
    }

    return true;
  };

  const register = async (email: string, password: string) => {
    try {
      // Mock API call
      await fetch('/api/register', {
        method: 'POST',
        headers: {
          'Content-Type': 'application/json',
        },
        body: JSON.stringify({ email, password }),
      });

      return true;
    } catch (error) {
      console.error('Registration failed:', error);
      return false;
    }
  };

  const logout = () => {
    setToken(null);
    sessionStorage.removeItem('jwt-token');
    navigate('/');
  };

  return { token, login, register, logout };
};

export default useAuth;