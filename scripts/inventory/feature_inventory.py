"""Snapshot Dove's user-facing surface from source, and diff two snapshots.

The UI revamp must not remove any feature. This script records what exists:
routes, side-panel tabs, settings sections, settings keys, rebindable and
fixed shortcuts, command-palette actions and every user-facing string in the
app, so a later snapshot can be diffed against the baseline. Anything that
disappears must be accounted for before a milestone merges.

usage:
  python -I scripts/inventory/feature_inventory.py snapshot <repo> <out.json>
  python -I scripts/inventory/feature_inventory.py diff <base.json> <new.json>

`diff` exits 1 when anything was removed.
"""

import json
import re
import sys
from pathlib import Path

APP = Path("apps/dove/src")
CRATES = Path("crates")


def rust_files(root: Path, rel: Path):
    return sorted(p for p in (root / rel).rglob("*.rs"))


def strip_tests(text: str) -> str:
    """Drop `#[cfg(test)]` modules so test fixtures don't count as features."""
    # Only a test module ends the file's features: `#[cfg(test)]` on a
    # single item (a test-only helper) must not cut what follows it.
    m = re.search(r"#\[cfg\(test\)\]\s*(pub(\(\w+\))?\s+)?mod ", text)
    return text if m is None else text[:m.start()]


def read(path: Path) -> str:
    return strip_tests(path.read_text(encoding="utf-8", errors="replace"))


def fn_body(text: str, name: str) -> str:
    """The text of the first `fn <name>` body (brace matched)."""
    m = re.search(rf"fn {name}\b[^{{]*\{{", text)
    if not m:
        return ""
    depth, i = 1, m.end()
    while i < len(text) and depth:
        depth += {"{": 1, "}": -1}.get(text[i], 0)
        i += 1
    return text[m.end():i]


def snapshot(repo: Path) -> dict:
    shell = read(repo / APP / "shell.rs")
    side_panel = read(repo / APP / "side_panel.rs")
    settings = read(repo / APP / "settings.rs")
    keymap = read(repo / APP / "keymap.rs")
    app_files = rust_files(repo, APP)
    all_files = app_files + rust_files(repo, CRATES)
    sources = {p: read(p) for p in all_files}

    inv: dict[str, list[str]] = {}
    route_re = r'"([a-z][\w-]*)"(?:\s*\|\s*"[\w-]+")*\s*=>\s*Route::(\w+)'
    inv["routes"] = sorted(f"{rid}={route}" for rid, route in re.findall(route_re, fn_body(shell, "from_id")))
    inv["panel_tabs"] = sorted(f"{v}={label}" for v, label in re.findall(r'Tab::(\w+)\s*=>\s*"([^"]+)"', fn_body(side_panel, "label")))
    inv["settings_sections"] = sorted(f"{v}={label}" for v, label in re.findall(r'Section::(\w+)\s*=>\s*"([^"]+)"', fn_body(settings, "label")))

    keys: set[str] = set()
    key_re = re.compile(r'(?:store\.setting|set_setting|setting_or|self\.set|self\.flag|\bflag)\(\s*&?"([a-z][a-z0-9_.]*)"')
    prefix_re = re.compile(r'format!\("([a-z][a-z_]*)\.\{')
    for p, text in sources.items():
        keys.update(key_re.findall(text))
        if "setting" in text:
            keys.update(f"{k}.*" for k in prefix_re.findall(text))
    inv["settings_keys"] = sorted(keys)

    inv["shortcuts"] = sorted(f"{i}|{label}|{keys}" for i, label, keys in re.findall(r'sc!\(\s*"(\w+)",\s*"([^"]+)",\s*"([^"]+)"', keymap))
    fixed = set()
    for p, text in sources.items():
        for keys_, action in re.findall(r'KeyBinding::new\(\s*&?(?:format!\()?"([^"]+)"\)?,\s*([\w:]+)', text):
            fixed.add(f"{keys_}|{action.split('::')[-1]}|{p.name}")
    inv["fixed_bindings"] = sorted(fixed)
    inv["palette_actions"] = sorted(set(f"{label}|{aid}" for label, aid in re.findall(r'P::Action\(\s*"([^"]+)",\s*[\w:]+,\s*"(\w+)"', shell)))

    # Every user-facing string literal in the app, per file: capitalised,
    # not a path, an identifier, a log target or a format-only string.
    lit = re.compile(r'"((?:[A-Z]|←|→)[^"\\]{1,120})"')
    skip = re.compile(r"^[A-Z0-9_]+$|^[A-Z][a-z]+[A-Z]|\.(svg|png|json|rs|ttf)$|^(Ctrl|Cmd|Shift|Alt)[-+]|::|^[A-Z]:\\\\")
    strings = set()
    for p in app_files:
        rel = p.relative_to(repo / APP).as_posix()
        for line in sources[p].splitlines():
            s = line.strip()
            if s.startswith(("//", "#[", "tracing::", "log::", "debug!", "warn!", "info!", "error!")):
                continue
            for m in lit.findall(line):
                if not skip.search(m):
                    strings.add(f"{rel}|{m}")
    inv["ui_strings"] = sorted(strings)
    return inv


def diff(base: dict, new: dict) -> int:
    removed_any = False
    for section in base:
        gone = sorted(set(base[section]) - set(new.get(section, [])))
        added = sorted(set(new.get(section, [])) - set(base[section]))
        if section == "ui_strings":
            # A string that only moved file is not a removal.
            def text_only(items):
                return {i.split("|", 1)[1] for i in items}
            gone = sorted(text_only(gone) - text_only(new.get(section, [])))
            added = sorted(text_only(added) - text_only(base[section]))
        print(f"## {section}: {len(base[section])} -> {len(new.get(section, []))} (removed {len(gone)}, added {len(added)})")
        for g in gone:
            print(f"  - {g}")
            removed_any = True
        for a in added[:50]:
            print(f"  + {a}")
        if len(added) > 50:
            print(f"  + ... {len(added) - 50} more")
    return 1 if removed_any else 0


def main():
    # Labels carry arrows and ellipses; print them on any console.
    sys.stdout.reconfigure(encoding="utf-8")
    if len(sys.argv) == 4 and sys.argv[1] == "snapshot":
        inv = snapshot(Path(sys.argv[2]))
        Path(sys.argv[3]).parent.mkdir(parents=True, exist_ok=True)
        Path(sys.argv[3]).write_text(json.dumps(inv, indent=1, ensure_ascii=False) + "\n", encoding="utf-8")
        for k, v in inv.items():
            print(f"{k}: {len(v)}")
    elif len(sys.argv) == 4 and sys.argv[1] == "diff":
        base = json.loads(Path(sys.argv[2]).read_text(encoding="utf-8"))
        new = json.loads(Path(sys.argv[3]).read_text(encoding="utf-8"))
        sys.exit(diff(base, new))
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main()
