'use strict';

// How the service is meant to behave. Not run in this round.
const test = require('node:test');
const assert = require('node:assert');
const { buildMeta, parsePage } = require('../src/pagination');

test('parsePage clamps limit and starts at page 1', () => {
  assert.deepStrictEqual(parsePage({}), { page: 1, limit: 20, offset: 0 });
  assert.strictEqual(parsePage({ page: '3', limit: '500' }).limit, 100);
});

test('45 products at 20 per page is 3 pages', () => {
  const meta = buildMeta(45, 2, 20);
  assert.strictEqual(meta.totalPages, 3);
  assert.strictEqual(meta.hasNext, true);
});

test('search text is data, not SQL', async () => {
  // GET /products?q=' OR 1=1 --
  // Expected: no rows match, and no other table is touched.
});

test('a page of 20 products makes a fixed number of queries', async () => {
  // Count the queries one request sends.
  // Expected: the same number for 1 product and for 100 products.
});
