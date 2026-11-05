"""Hörwerk – the audiobook workshop: inbox -> M4B -> Audible tags -> library.

Start: ``python main.py``. The pieces:
  config.py      environment variables and default settings
  services.py    wires the services (pipeline, converter, library index, usenet)
  web/           HTTP server, security checks, router and the API handlers
"""
from __future__ import annotations

import os
import signal
import threading

import applog
from config import VERSION, Config
from services import create_app
from web.server import make_server


def main() -> None:
    # Unraid & co: files we create get these permissions (UMASK=000 -> rw for everyone)
    if os.environ.get("UMASK"):
        os.umask(int(os.environ["UMASK"], 8))
    applog.capture_stderr()
    try:
        config = Config.from_env()
    except ValueError as e:                 # a setting Hörwerk cannot use: say which, don't start half-configured
        applog.error(f"Start abgebrochen: {e}", "Start")
        raise SystemExit(2)
    app = create_app(config)
    server = make_server(app)
    applog.info(f"Hörwerk {VERSION} läuft auf Port {config.port} – Eingang: {config.inbox}, Bibliothek: {config.library}")
    stopping = threading.Event()

    def on_signal(signum, frame) -> None:   # docker stop (SIGTERM) or Ctrl+C
        if stopping.is_set():
            return
        stopping.set()
        # serve_forever runs in this very thread: shutting it down has to happen in another one
        threading.Thread(target=server.shutdown, daemon=True, name="shutdown").start()

    signal.signal(signal.SIGTERM, on_signal)
    signal.signal(signal.SIGINT, on_signal)
    server.serve_forever()                  # returns once the server is shut down: no new requests from here on
    server.server_close()
    app.stop()


if __name__ == "__main__":
    main()
