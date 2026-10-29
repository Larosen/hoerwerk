"""API handlers, one module per area. Importing this package registers them."""
from web.routes import audiobooks, convert, files, library, search, system

__all__ = ["audiobooks", "convert", "files", "library", "search", "system"]
