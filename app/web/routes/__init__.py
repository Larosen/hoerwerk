"""API handlers, one module per area. Importing this package registers them."""
from web.routes import audiobooks, auth, discover, invites, library, logs, notify, search, system, requests, upload

__all__ = ["audiobooks", "auth", "discover", "invites", "library", "logs", "notify", "search", "system", "requests", "upload"]
