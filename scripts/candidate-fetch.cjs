// Test-only transport for the unmodified installed wrapper. Never shipped in the npm package.
const fs = require('node:fs');
const path = require('node:path');
const feed = JSON.parse(fs.readFileSync(process.env.FIGLENS_TEST_FEED, 'utf8'));
globalThis.fetch = async url => {
  const base = `https://github.com/runkids/figlens/releases/download/v${feed.version}/`;
  if (typeof url !== 'string' || !url.startsWith(base)) throw new Error(`Unexpected network request: ${url}`);
  const name = url.slice(base.length);
  if (![feed.archive, 'SHA256SUMS'].includes(name)) throw new Error(`Unexpected release asset: ${name}`);
  fs.appendFileSync(feed.events, JSON.stringify({ name }) + '\n');
  const bytes = fs.readFileSync(path.join(feed.directory, name));
  return { ok: true, status: 200, arrayBuffer: async () => bytes };
};
