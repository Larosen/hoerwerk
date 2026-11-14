#!/usr/bin/env python3
"""Prüft die Textkontraste (WCAG 2.2, Ziel ≥ 4,5:1) der drei Farbwelten im Prototyp: python3 docs/design/check_contrast.py"""
import os, re, sys
S = open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "prototype", "index.html"), encoding="utf-8").read()
THEMES = {"Hell": ':root, :root[data-theme="light"] {', "Dunkel": ':root[data-theme="dark"] {', "OLED": ':root[data-theme="oled"] {'}
PAIRS = [("ink","bg"),("muted","bg"),("muted","surface"),("faint","bg"),("faint","surface"),("accent","bg"),("accent","surface"),
         ("accent-ink","accent"),("brass","bg"),("brass","surface"),("ok","surface"),("warn","surface"),("err","surface"),
         ("warn","warn-soft"),("err","err-soft"),("ok","ok-soft"),("brass","brass-soft"),("accent","accent-soft")]
def lum(h):
    r, g, b = [int(h.lstrip("#")[i:i+2], 16) / 255 for i in (0, 2, 4)]
    f = lambda c: c / 12.92 if c <= .03928 else ((c + .055) / 1.055) ** 2.4
    return .2126 * f(r) + .7152 * f(g) + .0722 * f(b)
def ratio(a, b):
    x, y = sorted((lum(a), lum(b)), reverse=True); return (x + .05) / (y + .05)
tok = {}
for name, sel in THEMES.items():
    i = S.index(sel); tok[name] = dict(re.findall(r"--([a-z0-9-]+):(#[0-9A-Fa-f]{6})", S[i:S.index("}", i)]))
bad = 0
print("Paar".ljust(26), *[n.ljust(7) for n in tok])
for a, b in PAIRS:
    row = [ratio(tok[n][a], tok[n][b]) for n in tok]; bad += sum(x < 4.5 for x in row)
    print(f"{a} auf {b}".ljust(26), *[(f"{x:.1f}" + ("" if x >= 4.5 else "!")).ljust(7) for x in row])
print("Unter 4,5:1:", bad); sys.exit(1 if bad else 0)
