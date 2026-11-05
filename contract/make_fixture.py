"""Builds contract/fixtures/hoerwerk-7.13.db with the Python implementation (run once, the file is checked in).

A later backend must open this file as it is: same accounts and passwords, settings, requests, library."""
import os
import shutil
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "app"))

import config  # noqa: E402
import services  # noqa: E402

tmp = tempfile.mkdtemp()
for d in ("config", "inbox", "lib"):
    os.makedirs(os.path.join(tmp, d))
app = services.App(config.Config.from_env({"CONFIG_DIR": os.path.join(tmp, "config"), "INBOX_DIR": os.path.join(tmp, "inbox"),
                                           "LIBRARY_DIR": os.path.join(tmp, "lib")}), start_workers=False)
admin = app.accounts.create("Alt-Admin", "Schimmelreiter-Kaffee-713", "admin")
eva = app.accounts.create("Eva", "Kaffee Garten Hund Fahrrad", "user")
app.update_settings({"audio_quality": "original", "language": "de"})
app.requests.search_soon = lambda rid: None
app.requests.create(eva, {"asin": "B0ALTDB001", "title": "Aus der alten Datenbank", "authors": ["Anna Autorin"],
                          "language": "german", "runtime_min": 123})
app.db.close()
os.makedirs(os.path.join(HERE, "fixtures"), exist_ok=True)
shutil.copy(os.path.join(tmp, "config", "hoerwerk.db"), os.path.join(HERE, "fixtures", "hoerwerk-7.13.db"))
shutil.rmtree(tmp)
print("written")
