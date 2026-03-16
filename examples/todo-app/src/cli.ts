import { dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createApp } from './composition-root.js';

const __dirname = dirname(fileURLToPath(import.meta.url));
const storageDir = dirname(__dirname);

const app = createApp(storageDir);

const args = process.argv.slice(2);

if (args[0] === 'serve') {
  app.http.listen(3456);
} else {
  await app.cli.run(args);
}
