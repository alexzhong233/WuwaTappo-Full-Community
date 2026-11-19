import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const { parse } = require('./acorn.cjs');

export { parse };
