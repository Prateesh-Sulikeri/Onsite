"""Which limiter a caller gets. Keys that start with sk_live_ are paying customers."""
from limiter import TokenBucket


def plan_for(api_key: str) -> str:
    if api_key.startswith("sk_live_"):
        return "paid"
    return "free"


def build_limiters(redis_client) -> dict:
    return {
        "free": TokenBucket(redis_client, capacity=20, refill_per_sec=2, prefix="rl:free"),
        "paid": TokenBucket(redis_client, capacity=200, refill_per_sec=50, prefix="rl:paid"),
    }
