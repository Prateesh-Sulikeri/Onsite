"""Token bucket rate limiter backed by Redis. State is shared by every app server."""
import time


class TokenBucket:
    def __init__(self, redis_client, capacity: int, refill_per_sec: float, prefix: str = "rl"):
        self.redis = redis_client
        self.capacity = capacity
        self.refill_per_sec = refill_per_sec
        self.prefix = prefix

    def _key(self, client_id: str) -> str:
        return f"{self.prefix}:{client_id}"

    def allow(self, client_id: str, cost: int = 1) -> bool:
        key = self._key(client_id)
        now = time.time()

        state = self.redis.hgetall(key)
        if state:
            tokens = float(state[b"tokens"])
            last = float(state[b"ts"])
        else:
            tokens = float(self.capacity)
            last = now

        tokens = min(float(self.capacity), tokens + (now - last) * self.refill_per_sec)
        allowed = tokens >= cost
        if allowed:
            tokens -= cost

        self.redis.hset(key, mapping={"tokens": tokens, "ts": now})
        return allowed

    def retry_after(self, client_id: str, cost: int = 1) -> float:
        state = self.redis.hgetall(self._key(client_id))
        if not state:
            return 0.0
        missing = cost - float(state[b"tokens"])
        if missing <= 0:
            return 0.0
        return missing / self.refill_per_sec
