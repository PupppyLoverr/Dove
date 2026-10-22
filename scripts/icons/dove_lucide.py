"""Builds Dove's interface icons from Lucide: 1.75 stroke, softer corners, one format.

Every icon in apps/dove/assets/icons except the agent brand marks and dove-logo
comes from this script. To add one, map it in MAP and re-run:

    npm pack lucide-static@1.52.0 && tar xzf lucide-static-1.52.0.tgz
    python3 -I scripts/icons/dove_lucide.py package/icons apps/dove/assets/icons
"""
import math
import os
import re
import sys

LUCIDE, OUT = sys.argv[1:3]

STROKE = "1.75"
ROUND = 1.5   # corner radius multiplier for quarter-circle corners
MAX_R = 4.5

# Dove file name -> Lucide icon name. Brand marks and dove-logo are not listed and stay as they are.
MAP = {
    "action-build": "hammer", "action-configure": "settings-2", "action-debug": "bug",
    "action-lint": "list-checks", "action-play": "play", "action-test": "flask-conical",
    "add-circle": "circle-plus",
    "alt-arrow-down": "chevron-down", "alt-arrow-left": "chevron-left",
    "alt-arrow-right": "chevron-right", "alt-arrow-up": "chevron-up",
    "archive-minimalistic": "archive", "archive-up-minimalistic": "archive-restore",
    "arrow-down": "arrow-down", "arrow-left": "arrow-left", "arrow-right": "arrow-right",
    "arrow-up": "arrow-up", "arrow-up-right": "arrow-up-right",
    "bell": "bell", "bot": "bot", "calendar": "calendar", "chat-round-line": "message-circle",
    "check": "check", "checklist": "list-todo", "clock-circle": "clock", "close-circle": "circle-x",
    "close": "x", "cloud": "cloud", "collapse-arrows": "minimize-2", "command": "command",
    "copy": "copy", "danger-triangle": "triangle-alert", "document-add": "file-plus",
    "document": "file", "drag-handle": "grip-vertical", "expand-arrows": "maximize-2",
    "eye-closed": "eye-off", "eye": "eye", "fast-tier": "zap",
    "file-code": "file-code", "file-data": "file-braces", "file-image": "file-image",
    "file-markdown": "file-text", "file-style": "file-type", "file-tree": "list-tree",
    "floppy-disk": "save", "fold-vertical": "fold-vertical", "folder-tree": "folder-tree",
    "folder-with-files": "folder-open", "folder": "folder", "fork": "git-fork",
    "gallery": "image", "git-branch": "git-branch", "global": "globe", "globe": "globe",
    "hard-drive": "hard-drive", "home": "house", "info-circle": "info",
    "key-minimalistic": "key-round", "keyboard": "keyboard", "laptop": "laptop",
    "list": "text-align-start", "logout-2": "log-out", "magic-stick-3": "wand-sparkles",
    "magnifer": "search", "microphone-off": "mic-off", "microphone": "mic", "monitor": "monitor",
    "moon": "moon", "more-horizontal": "ellipsis", "palette-search": "search",
    "paperclip": "paperclip", "pen-new-square": "square-pen", "pen": "pencil",
    "phone-hang-up": "phone-off", "pin": "pin", "plus": "plus", "project-default": "code",
    "pull-request-draft": "git-pull-request-draft", "pull-request": "git-pull-request",
    "queue-check": "check", "queue-close": "x", "queue-drag-handle": "grip-vertical",
    "queue-paperclip": "paperclip", "queue-send": "arrow-right", "refresh": "refresh-cw",
    "remote-server": "server", "restart": "rotate-ccw", "return": "corner-down-left",
    "settings-minimalistic": "settings-2", "settings": "settings", "smartphone": "smartphone",
    "sort-vertical": "arrow-up-down", "sort": "list-filter", "split-columns": "columns-2",
    "star": "star", "sun": "sun", "tag": "tag", "terminal": "square-terminal",
    "trash-bin-minimalistic": "trash-2", "tuning": "sliders-horizontal", "users-group": "users",
    "volume-loud": "volume-2", "widget-add": "grid-2x2-plus", "widget": "layout-grid",
    "wifi-off": "wifi-off", "worktree": "folder-git", "wrap-text": "text-wrap",
    # filled variants, built from the outline icon below
    "star-bold": "star", "fast-tier-bold": "zap",
}
FILLED = {"star-bold", "fast-tier-bold"}

# Drawn here in the same style (Lucide has no window controls or filled stop).
CUSTOM = {
    "stop": '<rect x="4" y="4" width="16" height="16" rx="4.5" fill="currentColor" stroke="none"/>',
    "window-maximize": '<rect x="6.5" y="6.5" width="11" height="11" rx="3"/>',
    "window-minimize": '<path d="M6.5 12h11"/>',
    "window-restore": '<rect x="6.5" y="9" width="8.5" height="8.5" rx="2.5"/><path d="M9.5 6.5h5.5a3 3 0 0 1 3 3V15"/>',
}

NUM = r"-?(?:\d+\.?\d*|\.\d+)(?:e-?\d+)?"


def fmt(v):
    v = round(v, 3)
    s = f"{v:.3f}".rstrip("0").rstrip(".")
    return "0" if s in ("-0", "") else s


def parse_path(d):
    """Absolute segments: [cmd, [args...], end(x, y)] with cmd in M L C Q A Z."""
    toks = []
    for c, args in re.findall(r"([MmLlHhVvCcSsQqTtAaZz])([^MmLlHhVvCcSsQqTtAaZz]*)", d):
        toks.append(c)
        if c in "Aa":
            rest = args.strip()
            pat = re.compile(rf"\s*,?\s*({NUM})\s*,?\s*({NUM})\s*,?\s*({NUM})\s*,?\s*([01])\s*,?\s*([01])\s*,?\s*({NUM})\s*,?\s*({NUM})")
            while rest:
                m = pat.match(rest)
                toks.extend(m.groups())
                rest = rest[m.end():].strip()
        else:
            toks.extend(re.findall(NUM, args))
    i, cmd = 0, None
    x = y = sx = sy = 0.0
    segs = []
    prev_ctrl = None

    def nums(n):
        nonlocal i
        out = [float(t) for t in toks[i:i + n]]
        i += n
        return out

    while i < len(toks):
        if re.match(r"[A-Za-z]", toks[i]):
            cmd = toks[i]
            i += 1
            if cmd in "Zz":
                segs.append(["Z", [], (sx, sy)])
                x, y = sx, sy
                prev_ctrl = None
                continue
        rel = cmd.islower()
        c = cmd.upper()
        if c == "M":
            a, b = nums(2)
            x, y = (x + a, y + b) if rel else (a, b)
            sx, sy = x, y
            segs.append(["M", [], (x, y)])
            cmd = "l" if rel else "L"
            prev_ctrl = None
        elif c in "LHV":
            if c == "L":
                a, b = nums(2)
                x, y = (x + a, y + b) if rel else (a, b)
            elif c == "H":
                (a,) = nums(1)
                x = x + a if rel else a
            else:
                (b,) = nums(1)
                y = y + b if rel else b
            segs.append(["L", [], (x, y)])
            prev_ctrl = None
        elif c in "CS":
            if c == "C":
                p = nums(6)
            else:
                q = nums(4)
                if prev_ctrl and segs[-1][0] == "C":
                    r1 = (2 * x - prev_ctrl[0], 2 * y - prev_ctrl[1])
                else:
                    r1 = (x, y)
                p = [r1[0] - (x if rel else 0), r1[1] - (y if rel else 0)] + q
            if rel:
                p = [p[k] + (x if k % 2 == 0 else y) for k in range(6)]
            prev_ctrl = (p[2], p[3])
            x, y = p[4], p[5]
            segs.append(["C", p[:4], (x, y)])
        elif c in "QT":
            if c == "Q":
                p = nums(4)
                if rel:
                    p = [p[0] + x, p[1] + y, p[2] + x, p[3] + y]
            else:
                q = nums(2)
                ctrl = (2 * x - prev_ctrl[0], 2 * y - prev_ctrl[1]) if prev_ctrl and segs[-1][0] == "Q" else (x, y)
                p = [ctrl[0], ctrl[1], q[0] + (x if rel else 0), q[1] + (y if rel else 0)]
            prev_ctrl = (p[0], p[1])
            x, y = p[2], p[3]
            segs.append(["Q", p[:2], (x, y)])
        elif c == "A":
            rx, ry, rot, large, sweep, a, b = nums(7)
            x, y = (x + a, y + b) if rel else (a, b)
            segs.append(["A", [rx, ry, rot, large, sweep], (x, y)])
            prev_ctrl = None
    return segs


def write_path(segs):
    out = []
    for c, a, (x, y) in segs:
        if c == "Z":
            out.append("Z")
        elif c in "ML":
            out.append(f"{c}{fmt(x)} {fmt(y)}")
        elif c == "C":
            out.append("C" + " ".join(fmt(v) for v in a + [x, y]))
        elif c == "Q":
            out.append("Q" + " ".join(fmt(v) for v in a + [x, y]))
        elif c == "A":
            rx, ry, rot, large, sweep = a
            out.append(f"A{fmt(rx)} {fmt(ry)} {fmt(rot)} {int(large)} {int(sweep)} {fmt(x)} {fmt(y)}")
    return "".join(out)


def soften(d):
    """Grows quarter-circle corners (arcs or circle-like cubics) by ROUND.

    A corner grows only when the straight line on each side is long enough to
    give up the extra radius, or when that side is an open end of the path.
    """
    segs = parse_path(d)
    for k, seg in enumerate(segs):
        if k == 0 or seg[0] not in ("A", "C"):
            continue
        start, end = segs[k - 1][2], seg[2]
        dx, dy = end[0] - start[0], end[1] - start[1]
        r = abs(dx)
        if r < 0.5 or r > 3 or abs(abs(dy) - r) > 1e-3:
            continue
        sx, sy = math.copysign(1, dx), math.copysign(1, dy)
        if seg[0] == "A":
            rx, ry, _, large, sweep = seg[1]
            if abs(rx - r) > 1e-3 or abs(ry - r) > 1e-3 or large:
                continue
            # quarter corner: tangents are the two axes; sweep picks the order
            if (sx * sy > 0) == bool(sweep):
                u, v = (sx, 0.0), (0.0, sy)
            else:
                u, v = (0.0, sy), (sx, 0.0)
        else:
            c1, c2 = seg[1][:2], seg[1][2:]
            t1 = (c1[0] - start[0], c1[1] - start[1])
            t2 = (end[0] - c2[0], end[1] - c2[1])
            if abs(t1[0]) > 1e-3 and abs(t1[1]) < 1e-3 and abs(t2[1]) > 1e-3 and abs(t2[0]) < 1e-3:
                u, v = (sx, 0.0), (0.0, sy)
            elif abs(t1[1]) > 1e-3 and abs(t1[0]) < 1e-3 and abs(t2[0]) > 1e-3 and abs(t2[1]) < 1e-3:
                u, v = (0.0, sy), (sx, 0.0)
            else:
                continue
            # the curve must actually head along those tangents
            if t1[0] * u[0] + t1[1] * u[1] <= 0 or t2[0] * v[0] + t2[1] * v[1] <= 0:
                continue
        R = min(r * ROUND, MAX_R)
        delta = R - r
        prev = segs[k - 1]
        if prev[0] == "L":
            p0 = segs[k - 2][2]
            if math.dist(p0, start) < delta + 0.25:
                continue
            w = ((start[0] - p0[0]) / math.dist(p0, start), (start[1] - p0[1]) / math.dist(p0, start))
            if abs(w[0] - u[0]) > 1e-3 or abs(w[1] - u[1]) > 1e-3:
                continue
        elif prev[0] != "M":
            continue
        nxt = segs[k + 1] if k + 1 < len(segs) else None
        if nxt is not None and nxt[0] in ("L", "Z"):
            if math.dist(end, nxt[2]) < delta + 0.25:
                continue
            L = math.dist(end, nxt[2])
            w = ((nxt[2][0] - end[0]) / L, (nxt[2][1] - end[1]) / L)
            if abs(w[0] - v[0]) > 1e-3 or abs(w[1] - v[1]) > 1e-3:
                continue
        elif nxt is not None and nxt[0] != "M":
            continue
        zi = None
        if prev[0] == "M":
            for j in range(k + 1, len(segs)):
                if segs[j][0] == "M":
                    break
                if segs[j][0] == "Z":
                    zi = j
                    break
        if zi is not None:
            # closed subpath starting on this corner: its closing line comes in along u
            p0 = segs[zi - 1][2]
            Lz = math.dist(p0, start)
            if Lz < delta + 0.25:
                continue
            w = ((start[0] - p0[0]) / Lz, (start[1] - p0[1]) / Lz)
            if abs(w[0] - u[0]) > 1e-3 or abs(w[1] - u[1]) > 1e-3:
                continue
            segs[zi][2] = (start[0] - u[0] * delta, start[1] - u[1] * delta)
        sweep = 1 if u[0] * v[1] - u[1] * v[0] > 0 else 0
        prev[2] = (start[0] - u[0] * delta, start[1] - u[1] * delta)
        seg[0], seg[1] = "A", [R, R, 0, 0, sweep]
        seg[2] = (end[0] + v[0] * delta, end[1] + v[1] * delta)
    return write_path(segs)


def soften_rect(attrs):
    w, h = float(attrs.get("width", 0)), float(attrs.get("height", 0))
    r = attrs.get("rx") or attrs.get("ry")
    if not r:
        return attrs
    r = float(r)
    R = min(r * ROUND, MAX_R, w / 2, h / 2)
    attrs["rx"] = fmt(R)
    attrs.pop("ry", None)
    return attrs


def convert(svg, filled=False):
    svg = re.sub(r"<!--.*?-->", "", svg, flags=re.S)
    body = re.search(r"<svg\b[^>]*>(.*)</svg>", svg, flags=re.S).group(1)
    out = []
    for tag, attr_s in re.findall(r"<(\w+)\b([^>]*?)/?>", body):
        attrs = dict(re.findall(r'([\w:-]+)="([^"]*)"', attr_s))
        attrs.pop("class", None)
        if tag == "path":
            attrs["d"] = soften(attrs["d"])
        elif tag == "rect":
            attrs = soften_rect(attrs)
        if filled:
            attrs["fill"] = "currentColor"
        out.append(f"<{tag} " + " ".join(f'{k}="{v}"' for k, v in attrs.items()) + "/>")
    return "".join(out)


def wrap(inner):
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" '
        f'stroke-width="{STROKE}" stroke-linecap="round" stroke-linejoin="round">{inner}</svg>\n'
    )


os.makedirs(OUT, exist_ok=True)
for name, src in MAP.items():
    svg = open(os.path.join(LUCIDE, src + ".svg"), encoding="utf-8").read()
    open(os.path.join(OUT, name + ".svg"), "w", encoding="utf-8").write(wrap(convert(svg, name in FILLED)))
for name, inner in CUSTOM.items():
    open(os.path.join(OUT, name + ".svg"), "w", encoding="utf-8").write(wrap(inner))
print(len(MAP) + len(CUSTOM), "icons written to", OUT)
