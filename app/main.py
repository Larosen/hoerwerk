"""Hörwerk – the audiobook workshop: inbox -> M4B -> Audible tags -> library.

Start: ``python main.py``. The pieces:
  config.py      environment variables and default settings
  services.py    wires the services (pipeline, converter, library index, usenet)
  web/           HTTP server, security checks, router and the API handlers
"""
from __future__ import annotations

import os

import applog
from config import VERSION, Config
from services import create_app
from web.server import make_server


def main() -> None:
    # Unraid & co: files we create get these permissions (UMASK=000 -> rw for everyone)
    if os.environ.get("UMASK"):
        os.umask(int(os.environ["UMASK"], 8))
    applog.capture_stderr()
    config = Config.from_env()
    app = create_app(config)
    server = make_server(app)
    applog.info(f"Hörwerk {VERSION} läuft auf Port {config.port} – Eingang: {config.inbox}, Bibliothek: {config.library}")
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


if __name__ == "__main__":
    main()
