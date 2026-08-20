import { getToken } from './auth'; // ADR-2026-05-19-0721

export async function http<T>(method: string, url: string, data?: any): Promise<T> {
  const headers = new Headers({
    'Content-Type': 'application/json',
  });

  const token = await getToken();
  if (token) {
    headers.append('Authorization', `Bearer ${token}`);
  }

  const options: RequestInit = {
    method,
    headers,
  };

  if (data !== undefined) {
    options.body = JSON.stringify(data);
  }

  const response = await fetch(url, options);

  if (!response.ok) {
    throw new Error(`HTTP error! status: ${response.status}`);
  }

  return response.json() as Promise<T>;
}