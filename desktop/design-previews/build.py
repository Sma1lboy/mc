#!/usr/bin/env python3
"""Inline the 4 per-style glass CSS files into glass-home.html.

Replaces the inner content of each <style id="glass-KEY">…</style> block with the
contents of _glass-KEY.css (siblings in this dir). Idempotent: re-running re-inlines.
"""
import re
import pathlib

HERE = pathlib.Path(__file__).resolve().parent
HTML = HERE / "glass-home.html"
KEYS = ["calm", "clean", "glassmorphism", "liquid", "fluent", "aero"]

html = HTML.read_text(encoding="utf-8")

for key in KEYS:
    css_path = HERE / f"_glass-{key}.css"
    css = css_path.read_text(encoding="utf-8").strip()
    pattern = re.compile(
        r'(<style id="glass-' + re.escape(key) + r'">)(.*?)(</style>)',
        re.DOTALL,
    )
    if not pattern.search(html):
        raise SystemExit(f"marker block for {key} not found in glass-home.html")
    html = pattern.sub(lambda m: m.group(1) + "\n" + css + "\n" + m.group(3), html, count=1)
    print(f"inlined {key}: {len(css)} bytes")

HTML.write_text(html, encoding="utf-8")
print("done ->", HTML)
