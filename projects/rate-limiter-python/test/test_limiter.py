"""How the limiter is meant to behave. Not run in this round."""
from limiter import TokenBucket


def test_burst_then_block(redis_client):
    bucket = TokenBucket(redis_client, capacity=5, refill_per_sec=1)
    results = [bucket.allow("k") for _ in range(8)]
    assert results == [True] * 5 + [False] * 3


def test_concurrent_callers_share_the_budget(redis_client):
    # 50 threads call allow("k") at once on a bucket of capacity 10.
    # Expected: exactly 10 are allowed.
    pass


def test_servers_with_different_clocks_agree(redis_client):
    # Two app servers, one 3 seconds ahead of the other, alternate calls.
    # Expected: the refill rate is the same as with one server.
    pass


def test_idle_clients_do_not_stay_in_redis(redis_client):
    # 10,000 different api keys each make one call, then go quiet.
    # Expected: their keys are gone after the bucket would have refilled.
    pass
