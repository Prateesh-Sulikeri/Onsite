"""Thin wrapper over the message broker client."""


class PublishError(Exception):
    pass


class Publisher:
    def __init__(self, producer, timeout_s: float = 5.0):
        self.producer = producer
        self.timeout_s = timeout_s

    def publish(self, topic: str, key: str, payload: str) -> None:
        try:
            future = self.producer.send(topic, key=key.encode(), value=payload.encode())
            future.get(timeout=self.timeout_s)
        except Exception as exc:
            raise PublishError(f"publish to {topic} failed: {exc}") from exc
