"""Text contrast of the three colour schemes in static/style.css (WCAG 2.2 AA: at least 4.5:1 for every text pair)."""
import os
import re
import unittest

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
THEMES = {"light": ':root, :root[data-theme="light"] {', "dark": ':root[data-theme="dark"] {',
          "oled": ':root[data-theme="oled"] {'}
# (text, background) pairs the pages actually use
PAIRS = [("text", "bg"), ("text", "panel"), ("text", "panel-2"), ("muted", "bg"), ("muted", "panel"), ("faint", "bg"),
         ("faint", "panel"), ("accent", "bg"), ("accent", "panel"), ("accent-text", "accent"), ("brass", "bg"),
         ("brass", "panel"), ("ok", "panel"), ("warn", "panel"), ("err", "panel"), ("on-err", "err"),
         ("warn", "warn-soft"), ("err", "err-soft"), ("brass", "brass-soft"), ("accent", "accent-soft"),
         ("mark-text", "mark")]


def luminance(hex_colour: str) -> float:
    def channel(c: float) -> float:
        return c / 12.92 if c <= .03928 else ((c + .055) / 1.055) ** 2.4
    r, g, b = (int(hex_colour.lstrip("#")[i:i + 2], 16) / 255 for i in (0, 2, 4))
    return .2126 * channel(r) + .7152 * channel(g) + .0722 * channel(b)


def ratio(a: str, b: str) -> float:
    hi, lo = sorted((luminance(a), luminance(b)), reverse=True)
    return (hi + .05) / (lo + .05)


def tokens() -> dict:
    with open(os.path.join(ROOT, "static", "style.css"), encoding="utf-8") as fh:
        css = fh.read()
    out = {}
    for name, selector in THEMES.items():
        start = css.index(selector)
        block = css[start:css.index("}", start)]
        out[name] = {k: ("".join(c * 2 for c in v) if len(v) == 3 else v) for k, v in
                     re.findall(r"--([a-z0-9-]+):\s*#([0-9A-Fa-f]{6}|[0-9A-Fa-f]{3})\b", block)}
    return out


class Contrast(unittest.TestCase):
    def test_every_text_pair_reaches_aa(self):
        bad = []
        for theme, tok in tokens().items():
            for fg, bg in PAIRS:
                value = ratio(tok[fg], tok[bg])
                if value < 4.5:
                    bad.append(f"{theme}: {fg} on {bg} = {value:.2f}:1")
        self.assertEqual(bad, [], "text below 4.5:1")


if __name__ == "__main__":
    unittest.main()
