'use strict';

const express = require('express');
const { parsePage, buildMeta } = require('./pagination');

function createApp(repo) {
  const app = express();

  app.get('/products', async (req, res) => {
    try {
      const { page, limit, offset } = parsePage(req.query);
      const filters = { q: req.query.q, category: req.query.category };

      const rows = await repo.search({ ...filters, sort: req.query.sort, limit, offset });
      const total = await repo.count(filters);

      const items = [];
      for (const row of rows) {
        const tags = await repo.tagsFor(row.id);
        items.push({
          id: row.id,
          name: row.name,
          category: row.category,
          price: row.price_cents / 100,
          tags,
        });
      }

      res.json({ items, meta: buildMeta(total, page, limit) });
    } catch (err) {
      console.error('product search failed', err);
      res.status(500).json({ error: 'internal error' });
    }
  });

  return app;
}

module.exports = { createApp };
