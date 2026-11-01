import time
from collections import OrderedDict


class TTLCache:
    """Small in-process cache. Entries expire after ttl_seconds. At most max_size entries are kept;
    when it is full, the entry that was used least recently makes room."""

    def __init__(self, max_size, ttl_seconds, clock=time.monotonic):
        if max_size < 1:
            raise ValueError("max_size must be at least 1")
        self._max_size = max_size
        self._ttl = ttl_seconds
        self._clock = clock
        self._data = OrderedDict()  # key -> (value, expires_at)
        self.hits = 0
        self.misses = 0

    def get(self, key, default=None):
        item = self._data.get(key)
        if item is None:
            self.misses += 1
            return default
        value, expires_at = item
        if expires_at <= self._clock():
            del self._data[key]
            self.misses += 1
            return default
        self._data.move_to_end(key)
        self.hits += 1
        return value

    def put(self, key, value):
        if key in self._data:
            _, expires_at = self._data[key]
            self._data[key] = (value, expires_at)
            self._data.move_to_end(key)
            return
        if len(self._data) >= self._max_size:
            self._data.popitem(last=True)
        self._data[key] = (value, self._clock() + self._ttl)

    def delete(self, key):
        self._data.pop(key, None)

    def __len__(self):
        return len(self._data)
