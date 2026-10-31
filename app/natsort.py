"""Natural sort order: "2" before "10", case ignored (track and folder names)."""
from __future__ import annotations

import re


def natural_key(s: str):
    return [int(t) if t.isdigit() else t.lower() for t in re.split(r"(\d+)", s)]
