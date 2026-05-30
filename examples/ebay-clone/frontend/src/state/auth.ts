import { createSignal } from 'solid-js';

// Auth state management
export const [authToken, setAuthToken] = createSignal<string | null>(null);

// Initialize from sessionStorage
 load
if => {
  const token = sessionStorage.getItem('authToken');
  if (token) {
    setAuthToken(token);
  }
})();

// Clear auth state
export function clearAuth() {
  setAuthToken(null);
  sessionStorage.removeItem('authToken');
}

// Check authentication status
export function isAuthenticated(): {
  return !!authToken();
}