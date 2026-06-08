import { shortenUrl } from './composition-root.js';
const url = process.argv[2] ?? 'https://anthropic.com';
const code = shortenUrl.shorten(url);
console.log(`shorten  ${url} -> ${code}`);
console.log(`resolve  ${code} -> ${shortenUrl.resolve(code)}`);
