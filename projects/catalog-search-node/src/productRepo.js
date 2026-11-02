'use strict';

class ProductRepo {
  constructor(pool) {
    this.pool = pool;
  }

  buildWhere({ q, category }) {
    let where = 'WHERE active = true';
    if (q) {
      where += ` AND name ILIKE '%${q}%'`;
    }
    if (category) {
      where += ` AND category = '${category}'`;
    }
    return where;
  }

  async search({ q, category, sort, limit, offset }) {
    const order = sort || 'created_at DESC';
    const sql =
      'SELECT id, name, category, price_cents, created_at FROM products ' +
      `${this.buildWhere({ q, category })} ORDER BY ${order} LIMIT ${limit} OFFSET ${offset}`;
    const { rows } = await this.pool.query(sql);
    return rows;
  }

  async count({ q, category }) {
    const sql = `SELECT count(*)::int AS n FROM products ${this.buildWhere({ q, category })}`;
    const { rows } = await this.pool.query(sql);
    return rows[0].n;
  }

  async tagsFor(productId) {
    const { rows } = await this.pool.query('SELECT tag FROM product_tags WHERE product_id = $1 ORDER BY tag', [
      productId,
    ]);
    return rows.map((r) => r.tag);
  }
}

module.exports = { ProductRepo };
