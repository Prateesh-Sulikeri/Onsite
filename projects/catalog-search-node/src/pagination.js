'use strict';

const DEFAULT_LIMIT = 20;
const MAX_LIMIT = 100;

function toInt(value, fallback) {
  const n = Number.parseInt(value, 10);
  return Number.isNaN(n) ? fallback : n;
}

/** Reads ?page= and ?limit= from the query string. Pages start at 1. */
function parsePage(query) {
  const page = Math.max(1, toInt(query.page, 1));
  const limit = Math.min(MAX_LIMIT, Math.max(1, toInt(query.limit, DEFAULT_LIMIT)));
  return { page, limit, offset: (page - 1) * limit };
}

/** The paging block that goes into every list response. */
function buildMeta(total, page, limit) {
  const totalPages = Math.floor(total / limit);
  return {
    page,
    limit,
    total,
    totalPages,
    hasNext: page < totalPages,
    hasPrev: page > 1,
  };
}

module.exports = { parsePage, buildMeta, DEFAULT_LIMIT, MAX_LIMIT };
