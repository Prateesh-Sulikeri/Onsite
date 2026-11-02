"""How the cache is used. Not run in this round."""
import sys
import unittest

sys.path.insert(0, "src")

from cache import TTLCache


class FakeClock:
    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now


class TTLCacheTest(unittest.TestCase):
    def test_entry_expires(self):
        clock = FakeClock()
        cache = TTLCache(max_size=2, ttl_seconds=10, clock=clock)
        cache.put("a", 1)
        clock.now = 11
        self.assertIsNone(cache.get("a"))

    def test_least_recently_used_leaves_first(self):
        cache = TTLCache(max_size=2, ttl_seconds=10)
        cache.put("a", 1)
        cache.put("b", 2)
        cache.get("a")
        cache.put("c", 3)
        self.assertIsNone(cache.get("b"))
        self.assertEqual(cache.get("a"), 1)


if __name__ == "__main__":
    unittest.main()
