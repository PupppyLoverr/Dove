"""Keplorayx's themes for the Zeron-based app: Zeron's dark and light themes
with Keplorayx's Orbit night and dawn laid over them, so every token Zeron's
screens read exists, in Keplorayx's colours. Run from the repository root."""

import json
import re

BACKSLASH = chr(92)


def jsonc(path):
    s = open(path, encoding='utf-8').read()
    out, i, n, inside = [], 0, len(s), False
    while i < n:
        c = s[i]
        if inside:
            out.append(c)
            if c == BACKSLASH:
                out.append(s[i + 1])
                i += 2
                continue
            if c == '"':
                inside = False
            i += 1
            continue
        if c == '"':
            inside = True
            out.append(c)
            i += 1
            continue
        if s.startswith('//', i):
            while i < n and s[i] != '\n':
                i += 1
            continue
        out.append(c)
        i += 1
    return json.loads(re.sub(r',(\s*[}\]])', r'\1', ''.join(out)))


def merge(a, b):
    if isinstance(a, dict) and isinstance(b, dict):
        r = dict(a)
        for k, v in b.items():
            r[k] = merge(a[k], v) if k in a else v
        return r
    return b


# (Zeron's ids are kept, as its screens and templates name them; the
# colours in them are Keplorayx's. Built from Zeron's originals, read from
# the Zeron checkout beside this one.)
PAIRS = [
    ('zeron-dark', 'orbit-night', 'zeron-dark', 'night'),
    ('zeron-light', 'orbit-dawn', 'zeron-light', 'dawn'),
]
for zeron, orbit, name, title in PAIRS:
    z = jsonc(f'../zeron/apps/zgx/ui/ds/themes/{zeron}.json')
    o = jsonc(f'apps/keplorayx/ui/kx/themes/{orbit}.json')
    m = merge(z, o)
    m['name'] = name
    head = (
        f"// Keplorayx {title}: Zeron's {zeron} with Keplorayx's Orbit {title} laid\n"
        f"// over it (apps/keplorayx/ui/kx/themes/{orbit}.json, by tools/kx_themes.py):\n"
        "// every token Zeron's screens read, in Keplorayx's colours.\n"
    )
    open(f'zr/apps/zgx/ui/ds/themes/{name}.json', 'w', encoding='utf-8', newline='\n').write(
        head + json.dumps(m, indent='\t', ensure_ascii=False) + '\n'
    )
    print(name)
