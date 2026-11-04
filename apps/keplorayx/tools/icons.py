"""Build Keplorayx's icon library: ui/kx/icon_svgs.luau and assets/icons/.

Sources:
  - Dove's library (apps/dove/assets/icons): 16-unit glyphs, 1.25 stroke,
    round caps and joins; brand and agent marks drawn filled.
  - Keplorayx's own glyphs, drawn by hand below on the same grid.

What it does to every glyph:
  - one root: viewBox 0 0 16 16, no fill, currentColor stroke, round caps
    and joins, and a stroke-width the drawing replaces per size (optical
    weight: ui/kx/icon.luau);
  - numbers trimmed to two decimals and whitespace collapsed, so the
    shapes stay exactly where they were drawn;
  - drawn filled parts (fill="currentColor") keep their fill.
Marks (brands, agents) are kept as drawn, apart from their root's size.

Run from the repository root:
  python -I apps/keplorayx/tools/icons.py
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SOURCE = ROOT / "apps" / "dove" / "assets" / "icons"
APP = ROOT / "apps" / "keplorayx"
OUT_SVG = APP / "assets" / "icons"
OUT_LUAU = APP / "ui" / "kx" / "icon_svgs.luau"

GLYPH_ROOT = (
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16" fill="none" '
    'stroke="currentColor" stroke-width="1.25" stroke-linecap="round" stroke-linejoin="round">'
)

# Keplorayx's own: drawn on the 16 grid, centred optically, 1.25 stroke.
DOT = 'fill="currentColor" stroke="none"'
OWN: dict[str, str] = {
    # The mark: a body on its orbit around your work.
    "orbit": (
        '<ellipse cx="8" cy="8" rx="6.25" ry="3.25" transform="rotate(-24 8 8)"/>'
        '<circle cx="8" cy="8" r="1.5"/>'
        f'<circle cx="13.71" cy="5.46" r="1.1" {DOT}/>'
    ),
    # Every running agent at once.
    "constellation": (
        '<path d="M3.25 11.75 6.5 5.25 10.25 8.75 12.75 3.75"/>'
        f'<circle cx="3.25" cy="11.75" r="1.15" {DOT}/>'
        f'<circle cx="6.5" cy="5.25" r="1.15" {DOT}/>'
        f'<circle cx="10.25" cy="8.75" r="1.15" {DOT}/>'
        f'<circle cx="12.75" cy="3.75" r="1.15" {DOT}/>'
        '<path d="M10.25 8.75 12.5 12.5" stroke-dasharray="0.01 2"/>'
    ),
    # The Attention inbox: one queue of what needs you.
    "inbox": (
        '<path d="M2.75 8.75 4.4 4.1C4.66 3.37 5.35 2.75 6.13 2.75H9.87C10.65 2.75 11.34 3.37 11.6 4.1L13.25 8.75"/>'
        '<path d="M2.75 8.75V11C2.75 12.24 3.76 13.25 5 13.25H11C12.24 13.25 13.25 12.24 13.25 11V8.75H10.25L9.5 10.25H6.5L5.75 8.75H2.75Z"/>'
    ),
    # The Ray bar: one bar for everything.
    "ray": (
        '<rect x="1.75" y="4.75" width="12.5" height="6.5" rx="3.25"/>'
        '<path d="M5.25 6.75 6.75 8 5.25 9.25"/>'
        '<path d="M8.5 9.25H10.75"/>'
    ),
    # A turn told as beats: a spine of what happened.
    "beats": (
        '<path d="M4 4.75V11.25"/>'
        f'<circle cx="4" cy="3.5" r="1.25" {DOT}/>'
        '<circle cx="4" cy="8" r="1.25"/>'
        f'<circle cx="4" cy="12.5" r="1.25" {DOT}/>'
        '<path d="M7.25 3.5H13.25M7.25 8H11.25M7.25 12.5H12.25"/>'
    ),
    # The Review deck: changes as a stack of hunk cards.
    "review-deck": (
        '<rect x="2.75" y="5.25" width="10.5" height="8" rx="2.25"/>'
        '<path d="M4.25 3.25H11.75"/>'
        '<path d="M5.25 8.25H8.25M5.25 10.5H10.75"/>'
    ),
    # Peek: a live preview over what you point at.
    "peek": (
        '<path d="M4.75 2.75H11.25C12.35 2.75 13.25 3.65 13.25 4.75V8.25C13.25 9.35 12.35 10.25 11.25 10.25H9.25L8 11.75L6.75 10.25H4.75C3.65 10.25 2.75 9.35 2.75 8.25V4.75C2.75 3.65 3.65 2.75 4.75 2.75Z"/>'
        '<path d="M5.5 6.5C6.1 5.5 7 5 8 5C9 5 9.9 5.5 10.5 6.5C9.9 7.5 9 8 8 8C7 8 6.1 7.5 5.5 6.5Z"/>'
        '<path d="M5 13.75H11"/>'
    ),
    # A teammate: a person with their own orbit.
    "teammate": (
        '<circle cx="8" cy="6" r="2.25"/>'
        '<path d="M4.25 12.75C4.75 10.85 6.2 9.75 8 9.75C9.8 9.75 11.25 10.85 11.75 12.75"/>'
        '<path d="M2.6 4.4C3.6 2.45 5.65 1.25 8 1.25C10.35 1.25 12.4 2.45 13.4 4.4" stroke-dasharray="0.01 2.1"/>'
    ),
    # Statuses: quiet shapes, the colour says the rest.
    "status-idle": '<circle cx="8" cy="8" r="2.5"/>',
    "status-working": (
        '<path d="M8 2.75A5.25 5.25 0 1 1 2.75 8"/>'
        f'<circle cx="8" cy="8" r="1.25" {DOT}/>'
    ),
    "status-needs-you": (
        '<circle cx="8" cy="8" r="5.25"/>'
        f'<circle cx="8" cy="8" r="2" {DOT}/>'
    ),
    "status-done": '<circle cx="8" cy="8" r="5.25"/><path d="M5.75 8.1 7.3 9.6 10.25 6.5"/>',
    "status-failed": '<circle cx="8" cy="8" r="5.25"/><path d="M8 5.25V8.5"/><path d="M8 10.75V10.76" stroke-width="1.6"/>',
    # The notch.
    "notch": (
        '<path d="M1.75 4.25H4.5C5.2 4.25 5.6 4.6 5.85 5.25L6.1 5.9C6.4 6.6 6.9 7 7.65 7H8.35C9.1 7 9.6 6.6 9.9 5.9L10.15 5.25C10.4 4.6 10.8 4.25 11.5 4.25H14.25"/>'
        '<rect x="1.75" y="2.25" width="12.5" height="11.5" rx="2.75"/>'
    ),
    # A shortcut: the key, not the letter.
    "hotkey": (
        '<rect x="2.25" y="4.25" width="11.5" height="7.5" rx="2"/>'
        '<path d="M4.75 6.75H5.25M7.75 6.75H8.25M10.75 6.75H11.25M5.75 9.25H10.25"/>'
    ),
    # Privacy: a lock held on this device.
    "local-only": (
        '<rect x="2.25" y="3.25" width="11.5" height="7.5" rx="1.75"/>'
        '<path d="M5.5 13.25H10.5M8 10.75V13.25"/>'
        '<path d="M6.75 6.75V6.25C6.75 5.56 7.31 5 8 5C8.69 5 9.25 5.56 9.25 6.25V6.75"/>'
        '<rect x="6.25" y="6.75" width="3.5" height="2.25" rx="0.75"/>'
    ),
    # The accent picker's stars.
    "star-four": '<path d="M8 2.25C8.35 5.6 10.4 7.65 13.75 8C10.4 8.35 8.35 10.4 8 13.75C7.65 10.4 5.6 8.35 2.25 8C5.6 7.65 7.65 5.6 8 2.25Z"/>',
}

NUM = re.compile(r"-?\d+\.\d{3,}")


def trim(s: str) -> str:
    """Numbers to two decimals; whitespace collapsed between tags."""
    s = NUM.sub(lambda m: f"{float(m.group()):.2f}".rstrip("0").rstrip("."), s)
    s = re.sub(r">\s+<", "><", s)
    return re.sub(r"\s{2,}", " ", s).strip()


def body(svg: str) -> str:
    m = re.search(r"<svg[^>]*>(.*)</svg>", svg, re.S)
    if not m:
        raise ValueError("no <svg> root")
    return m.group(1)


def is_mark(name: str, svg: str) -> bool:
    return name.endswith("-mark") or name.startswith("brand-") or 'stroke-width="1.25"' not in svg


def main() -> int:
    glyphs: dict[str, str] = {}
    marks: dict[str, str] = {}
    for path in sorted(SOURCE.glob("*.svg")):
        name = path.stem
        svg = path.read_text(encoding="utf-8")
        if is_mark(name, svg):
            marks[name] = trim(svg)
        else:
            inner = body(svg)
            # A part drawn thicker or thinner keeps its own width; the root's
            # is the one the drawing sets.
            glyphs[name] = trim(GLYPH_ROOT + inner + "</svg>")
    for name, inner in OWN.items():
        glyphs[name] = trim(GLYPH_ROOT + inner + "</svg>")

    for kind, table in (("glyphs", glyphs), ("marks", marks)):
        folder = OUT_SVG / kind
        folder.mkdir(parents=True, exist_ok=True)
        for old in folder.glob("*.svg"):
            old.unlink()
        for name, svg in table.items():
            (folder / f"{name}.svg").write_text(svg + "\n", encoding="utf-8")

    lines = [
        "--!strict",
        "-- Generated by tools/icons.py from apps/dove/assets/icons and the",
        "-- glyphs drawn there: don't edit. Each icon's SVG by name, drawn as",
        "-- `code:` (ui/kx/icon.luau), so a plugin lent the design system draws",
        "-- them too.",
        "",
        "local M = {}",
        "",
        "M.glyphs = {",
    ]
    for name, svg in glyphs.items():
        lines.append(f'\t["{name}"] = [==[{svg}]==],')
    lines += ["}", "", "M.marks = {"]
    for name, svg in marks.items():
        lines.append(f'\t["{name}"] = [==[{svg}]==],')
    lines += ["}", "", "return M", ""]
    OUT_LUAU.write_text("\n".join(lines), encoding="utf-8")
    print(f"{len(glyphs)} glyphs ({len(OWN)} drawn for Keplorayx), {len(marks)} marks")
    return 0


if __name__ == "__main__":
    sys.exit(main())
