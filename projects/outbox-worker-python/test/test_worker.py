"""How the worker is meant to behave. Not run in this round."""
from outbox import Message
from publisher import PublishError
from worker import OutboxWorker


class FakeRepo:
    def __init__(self, messages):
        self.messages = messages
        self.sent = []
        self.failed = []

    def fetch_pending(self, limit):
        return self.messages[:limit]

    def mark_sent(self, msg_id):
        self.sent.append(msg_id)

    def mark_failed(self, msg_id, error):
        self.failed.append(msg_id)


class DownPublisher:
    def publish(self, topic, key, payload):
        raise PublishError("broker down")


def test_failed_publish_is_not_marked_sent():
    repo = FakeRepo([Message(1, "payments", "o1", "{}", 0)])
    OutboxWorker(repo, DownPublisher()).run_once()
    assert repo.sent == []
    assert repo.failed == [1]


def test_two_workers_do_not_publish_the_same_row():
    # Needs a real Postgres: two workers call fetch_pending at the same time.
    # Expected: each row is returned to exactly one of them.
    pass


def test_poison_row_stops_retrying():
    # A row whose publish always fails.
    # Expected: after a few attempts it leaves the queue and an alert can fire.
    pass
