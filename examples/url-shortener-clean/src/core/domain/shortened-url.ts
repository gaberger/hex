export interface ShortenedUrl {
  code: string;
  longUrl: string;
}

const ALPHABET = '0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ';

export function generateShortCode(seed: number): string {
  let n = Math.floor(seed);
  if (n <= 0) {
    return ALPHABET[0];
  }
  let code = '';
  while (n > 0) {
    code = ALPHABET[n % 62] + code;
    n = Math.floor(n / 62);
  }
  return code;
}
