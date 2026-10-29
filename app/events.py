"""Live updates for the browser (Server-Sent Events).

Services call ``bus.publish("audiobooks")`` whenever something changes. Every open
browser tab holds one ``/api/events`` connection and gets the new state of that topic
pushed – at most every ``interval`` seconds per topic, so a running conversion with
many progress ticks costs the same as a quiet one. Without changes the connection
only carries a heartbeat every 15 s.
"""
from __future__ import annotations

import json
import threading
import time
from typing import Callable, Dict, Iterator, List, Optional


class EventBus:
    def __init__(self, interval: float = 0.5, max_clients: int = 32) -> None:
        self.interval = interval
        self.max_clients = max_clients
        self.providers: Dict[str, Callable[[], object]] = {}
        self.versions: Dict[str, int] = {}
        self.cond = threading.Condition()
        self.clients = 0
        self.closed = False
        self._cache: Dict[str, tuple] = {}     # topic -> (version, json)

    def provide(self, topic: str, fn: Callable[[], object]) -> None:
        self.providers[topic] = fn
        self.versions.setdefault(topic, 0)

    def publish(self, topic: str) -> None:
        with self.cond:
            self.versions[topic] = self.versions.get(topic, 0) + 1
            self.cond.notify_all()

    def notifier(self, topic: str) -> Callable[[], None]:
        return lambda: self.publish(topic)

    def close(self) -> None:
        with self.cond:
            self.closed = True
            self.cond.notify_all()

    def snapshot(self, topic: str) -> str:
        """The topic's current state as JSON (computed once per version, shared by all clients)."""
        version = self.versions.get(topic, 0)
        cached = self._cache.get(topic)
        if cached and cached[0] == version:
            return cached[1]
        text = json.dumps(self.providers[topic](), ensure_ascii=False, separators=(",", ":"))
        self._cache[topic] = (version, text)
        return text

    def _wait(self, seen: Dict[str, int], topics: List[str], timeout: float) -> Optional[List[str]]:
        """Topics that changed since ``seen``; [] after the timeout; None when the bus is closed."""
        end = time.monotonic() + timeout
        with self.cond:
            while True:
                if self.closed:
                    return None
                changed = [t for t in topics if self.versions.get(t, 0) != seen.get(t)]
                if changed:
                    return changed
                left = end - time.monotonic()
                if left <= 0:
                    return []
                self.cond.wait(left)

    def stream(self, topics: List[str], heartbeat: float = 15.0) -> "Stream":
        """One SSE connection (raises OverflowError when too many are open)."""
        with self.cond:
            if self.clients >= self.max_clients:
                raise OverflowError("Zu viele offene Verbindungen")
            self.clients += 1
        return Stream(self, topics, heartbeat)

    def _release(self) -> None:
        with self.cond:
            self.clients -= 1


class Stream:
    """The bytes of one SSE connection: first the full state, then every change."""

    def __init__(self, bus: EventBus, topics: List[str], heartbeat: float) -> None:
        self.bus, self.topics, self.heartbeat = bus, topics, heartbeat
        self._open = True

    def __iter__(self) -> Iterator[bytes]:
        bus = self.bus
        yield b"retry: 3000\n\n"
        seen: Dict[str, int] = {}
        pending: Optional[List[str]] = list(self.topics)
        while self._open:
            for topic in pending or []:
                seen[topic] = bus.versions.get(topic, 0)
                yield f"event: {topic}\ndata: {bus.snapshot(topic)}\n\n".encode()
            time.sleep(bus.interval)            # throttle: collect further changes
            pending = bus._wait(seen, self.topics, self.heartbeat)
            if pending is None:
                return
            if not pending:
                yield b": ping\n\n"

    def close(self) -> None:
        if self._open:
            self._open = False
            self.bus._release()
