"""Postgres access for the outbox table."""
from dataclasses import dataclass
from typing import List


@dataclass
class Message:
    id: int
    topic: str
    key: str
    payload: str
    attempts: int


class OutboxRepo:
    def __init__(self, pool):
        self.pool = pool

    def fetch_pending(self, limit: int) -> List[Message]:
        with self.pool.connection() as conn:
            rows = conn.execute(
                "SELECT id, topic, key, payload, attempts FROM outbox "
                "WHERE status = 'pending' AND available_at <= now() "
                "ORDER BY id LIMIT %s",
                (limit,),
            ).fetchall()
        return [Message(*row) for row in rows]

    def mark_sent(self, msg_id: int) -> None:
        with self.pool.connection() as conn:
            conn.execute(
                "UPDATE outbox SET status = 'sent', sent_at = now() WHERE id = %s",
                (msg_id,),
            )

    def mark_failed(self, msg_id: int, error: str) -> None:
        with self.pool.connection() as conn:
            conn.execute(
                "UPDATE outbox SET attempts = attempts + 1, last_error = %s WHERE id = %s",
                (error[:500], msg_id),
            )

    def depth(self) -> int:
        with self.pool.connection() as conn:
            row = conn.execute("SELECT count(*) FROM outbox WHERE status = 'pending'").fetchone()
        return row[0]
