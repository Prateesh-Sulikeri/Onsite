import threading

from cache import TTLCache


class ProfileService:
    """Serves user profiles to many request threads. The database is slow, so profiles are cached
    for a minute."""

    def __init__(self, db, clock=None):
        self._db = db
        kwargs = {"clock": clock} if clock else {}
        self._cache = TTLCache(max_size=1000, ttl_seconds=60, **kwargs)
        self._lock = threading.Lock()

    def get_profile(self, user_id):
        profile = self._cache.get(user_id)
        if profile is not None:
            return profile
        profile = self._db.load_profile(user_id)
        self._cache.put(user_id, profile)
        return profile

    def update_display_name(self, user_id, name):
        self._db.save_display_name(user_id, name)
        with self._lock:
            profile = self._cache.get(user_id)
            if profile is not None:
                profile["display_name"] = name

    def cached_count(self):
        return len(self._cache)
