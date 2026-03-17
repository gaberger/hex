import { compose } from './composition-root.js';

const app = compose();
await app.run(process.argv.slice(2));
