"""Moves rows from the outbox table to the broker."""
import logging
import time

from outbox import OutboxRepo
from publisher import PublishError, Publisher

log = logging.getLogger("outbox.worker")


class OutboxWorker:
    def __init__(self, repo: OutboxRepo, publisher: Publisher, batch_size: int = 50, idle_sleep: float = 1.0):
        self.repo = repo
        self.publisher = publisher
        self.batch_size = batch_size
        self.idle_sleep = idle_sleep
        self.stopped = False

    def run_once(self) -> int:
        messages = self.repo.fetch_pending(self.batch_size)
        for msg in messages:
            self.repo.mark_sent(msg.id)
            try:
                self.publisher.publish(msg.topic, msg.key, msg.payload)
            except PublishError as exc:
                log.warning("message %s failed: %s", msg.id, exc)
                self.repo.mark_failed(msg.id, str(exc))
        return len(messages)

    def run_forever(self) -> None:
        while not self.stopped:
            handled = self.run_once()
            if handled == 0:
                time.sleep(self.idle_sleep)

    def stop(self) -> None:
        self.stopped = True
