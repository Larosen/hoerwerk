"""Builds contract/settings.json from app/config.py: default settings, allowed values and which environment
variable sets which setting (with its type). Run after a deliberate change of the settings."""
import json
import os
import re
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, "..", "app"))

import audible  # noqa: E402
import config  # noqa: E402


def build() -> dict:
    with open(os.path.join(HERE, "..", "app", "config.py"), encoding="utf-8") as fh:
        src = fh.read()
    body = src[src.index("def default_settings"):src.index("CHOICES = {")]
    env = {}
    for key, expr in re.findall(r'"(\w+)":\s*(.+?),\s*(?:#.*)?$', body, re.M):
        m = re.search(r'b\("(\w+)",\s*(True|False)\)', expr)
        if m:
            env[m.group(1)] = {"key": key, "type": "bool"}
            continue
        m = re.search(r'(int|float)\(env\.get\("(\w+)"', expr)
        if m:
            env[m.group(2)] = {"key": key, "type": m.group(1)}
            continue
        m = re.search(r'env\.get\("(\w+)"', expr)
        if m:
            env[m.group(1)] = {"key": key, "type": "str", **({"lower": True} if ".lower()" in expr else {})}
    defaults = config.default_settings({})
    if not all(v["key"] in defaults for v in env.values()):
        raise ValueError("unknown key in env map")
    out = {"description": "Settings of Hörwerk: defaults without environment, allowed values, and the environment "
                          "variables that change a default (type: bool = true/1/yes/on).",
           "defaults": defaults, "choices": {k: list(v) for k, v in {**config.CHOICES, "ab_region": tuple(audible.REGIONS)}.items()},
           "env": dict(sorted(env.items())), "version": config.VERSION}
    return out


if __name__ == "__main__":
    out = build()
    with open(os.path.join(HERE, "settings.json"), "w", encoding="utf-8") as fh:
        json.dump(out, fh, ensure_ascii=False, indent=1)
        fh.write("\n")
    print(len(out["defaults"]), "settings,", len(out["env"]), "variables")
