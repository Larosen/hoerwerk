"""API handlers, one module per area. Importing this package registers them."""
from web.routes import audiobooks, library, logs, notify, search, system

__all__ = ["audiobooks", "library", "logs", "notify", "search", "system"]
