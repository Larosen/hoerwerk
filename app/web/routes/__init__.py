"""API handlers, one module per area. Importing this package registers them."""
from web.routes import audiobooks, discover, library, logs, notify, search, system, wanted

__all__ = ["audiobooks", "discover", "library", "logs", "notify", "search", "system", "wanted"]
