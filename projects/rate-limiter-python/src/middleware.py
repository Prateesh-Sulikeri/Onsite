"""Applies the limiter to incoming requests."""
from typing import Dict, Tuple

from limiter import TokenBucket
from plans import plan_for


class RateLimitMiddleware:
    def __init__(self, limiters: Dict[str, TokenBucket]):
        self.limiters = limiters

    def check(self, api_key: str, route: str) -> Tuple[int, Dict[str, str]]:
        plan = plan_for(api_key)
        limiter = self.limiters[plan]
        client_id = f"{api_key}:{route}"
        if limiter.allow(client_id):
            return 200, {}
        wait = limiter.retry_after(client_id)
        return 429, {"Retry-After": str(int(wait) + 1)}
