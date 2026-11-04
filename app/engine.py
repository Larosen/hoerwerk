"""One job engine for everything that runs in the background.

Work is split into steps; every step goes to a queue with its own workers:

* ``encode`` – ffmpeg (M4B conversion). One worker by default: a conversion already
  uses all configured CPU cores, two at once would only fight over them.
* ``io``     – network and file work (Audible lookup, tags, cover, filing into the
  library). Short steps that must not wait behind a two-hour conversion.

A step is a callable; the job kinds (audiobooks, folder conversions) decide which
step comes next and persist their own state. The engine only schedules.
"""
from __future__ import annotations

import applog
import threading
import time
from collections import deque
from typing import Callable, Deque, Dict, List, Optional, Tuple

QUEUES = ("encode", "io")


class Engine:
    def __init__(self, workers: Optional[Dict[str, int]] = None,
                 on_change: Optional[Callable[[], None]] = None) -> None:
        self.workers = {"encode": 1, "io": 2, **(workers or {})}
        self.on_change = on_change
        self.lock = threading.Lock()
        self.wake = threading.Condition(self.lock)
        self.queues: Dict[str, Deque[Tuple[str, Callable[[], None]]]] = {q: deque() for q in QUEUES}
        self.running: Dict[str, List[str]] = {q: [] for q in QUEUES}   # keys of the steps being worked on
        self.done = 0
        self._started = False

    def start(self) -> None:
        with self.lock:
            if self._started:
                return
            self._started = True
        for q in QUEUES:
            for n in range(max(1, int(self.workers.get(q) or 1))):
                threading.Thread(target=self._worker, args=(q,), daemon=True, name=f"{q}-{n + 1}").start()

    def submit(self, queue: str, key: str, step: Callable[[], None]) -> bool:
        """Queue a step; a key that is already waiting is not queued twice."""
        if queue not in self.queues:
            raise ValueError(f"unknown queue: {queue}")
        with self.lock:
            if any(k == key for k, _ in self.queues[queue]):
                return False
            self.queues[queue].append((key, step))
            self.wake.notify_all()
        self._changed()
        return True

    def discard(self, key: str) -> None:
        """Drop waiting steps of a job (cancelled or removed)."""
        with self.lock:
            for q in self.queues.values():
                for item in [i for i in q if i[0] == key]:
                    q.remove(item)

    def is_waiting(self, key: str) -> bool:
        with self.lock:
            return any(k == key for q in self.queues.values() for k, _ in q)

    def stats(self) -> dict:
        with self.lock:
            return {q: {"workers": self.workers.get(q, 1), "running": len(self.running[q]),
                        "waiting": len(self.queues[q])} for q in QUEUES}

    def idle(self) -> bool:
        with self.lock:
            return not any(self.queues.values()) and not any(self.running.values())

    def wait_idle(self, timeout: float = 30) -> bool:
        """For tests: block until nothing is queued or running."""
        end = time.time() + timeout
        while time.time() < end:
            if self.idle():
                return True
            time.sleep(0.05)
        return False

    # -- internals ---------------------------------------------------------
    def _changed(self) -> None:
        if self.on_change:
            try:
                self.on_change()
            except Exception:  # noqa: BLE001 – a listener must never stop the engine
                pass

    def _next(self, queue: str) -> Optional[Tuple[str, Callable[[], None]]]:
        """The first waiting step whose job is not being worked on right now (in any queue) – two steps of one
        job never run at the same time, even when it was queued again while running."""
        busy = {k for keys in self.running.values() for k in keys}
        for item in self.queues[queue]:
            if item[0] not in busy:
                self.queues[queue].remove(item)
                return item
        return None

    def _worker(self, queue: str) -> None:
        while True:
            with self.lock:
                item = self._next(queue)
                while item is None:
                    self.wake.wait()
                    item = self._next(queue)
                key, step = item
                self.running[queue].append(key)
            try:
                step()
            except Exception as e:  # noqa: BLE001 – steps report their own errors; this is a last resort
                applog.error(f"Fehler in Warteschlange {queue} ({key}): {e!r}", "engine")
            finally:
                with self.lock:
                    self.running[queue].remove(key)
                    self.done += 1
                    self.wake.notify_all()           # a step held back for this job may start now
                self._changed()
