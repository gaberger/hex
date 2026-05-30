// Minimal token + API-base helpers. The in-memory backend issues a bearer
// token == username (UsernameTokenIssuer), so storing it here is sufficient to
// authenticate subsequent requests via the `http` client.

const TOKEN_KEY = 'ebay-token';

export const API_BASE: string =
  (import.meta.env.VITE_API as string | undefined) ?? 'http://localhost:8080';

export function getToken(): string | null {
  return localStorage.getItem(TOKEN_KEY);
}

export function setToken(token: string): void {
  localStorage.setItem(TOKEN_KEY, token);
}

export function clearToken(): void {
  localStorage.removeItem(TOKEN_KEY);
}
