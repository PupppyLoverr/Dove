#!/usr/bin/env python3
# /// script
# requires-python = ">=3.9"
# dependencies = ["python-pptx>=1.0"]
# ///
"""Create, inspect and edit .pptx decks. See ../SKILL.md."""
import argparse
import copy
import json
import os
import re
import sys

try:
    from pptx import Presentation
    from pptx.dml.color import RGBColor
    from pptx.enum.shapes import MSO_SHAPE
    from pptx.enum.text import PP_ALIGN
    from pptx.util import Emu, Inches, Pt
except ImportError:
    sys.exit("python-pptx is missing: run with `uv run` or `python3 -m pip install --user python-pptx`")

WIDE = (Inches(13.333), Inches(7.5))

# Restrained, office-grade looks. One accent, near-black ink, generous
# whitespace; every font is a theme font so the deck stays editable.
THEMES = {
    "modern": {"heading": "Calibri Light", "body": "Calibri", "ink": "1F2328", "muted": "5B6470", "accent": "2F5597", "light": "F2F4F7", "rule": "D0D5DD"},
    "classic": {"heading": "Georgia", "body": "Calibri", "ink": "1A1A1A", "muted": "5F6368", "accent": "7A2E3A", "light": "F5F2EE", "rule": "D9D3CC"},
    "mono": {"heading": "Arial", "body": "Arial", "ink": "111111", "muted": "666666", "accent": "111111", "light": "F2F2F2", "rule": "CCCCCC"},
}

A = "http://schemas.openxmlformats.org/drawingml/2006/main"
P = "http://schemas.openxmlformats.org/presentationml/2006/main"


CURRENT = THEMES["modern"]


def _q(ns, tag):
    return f"{{{ns}}}{tag}"


def _solid(parent, hexv):
    for c in list(parent):
        parent.remove(c)
    el = parent.makeelement(_q(A, "srgbClr"), {"val": hexv})
    parent.append(el)


def _lvl_style(lvl, size, color, bold=False, space_before=None, align=None):
    """Set a master text level's default run size/colour and spacing."""
    if align:
        lvl.set("algn", align)
    if space_before is not None:
        for old in lvl.findall(_q(A, "spcBef")):
            lvl.remove(old)
        spc = lvl.makeelement(_q(A, "spcBef"), {})
        spc.append(spc.makeelement(_q(A, "spcPts"), {"val": str(int(space_before * 100))}))
        lvl.insert(0, spc)
    rpr = lvl.find(_q(A, "defRPr"))
    if rpr is None:
        rpr = lvl.makeelement(_q(A, "defRPr"), {})
        lvl.append(rpr)
    rpr.set("sz", str(int(size * 100)))
    rpr.set("b", "1" if bold else "0")
    fill = rpr.find(_q(A, "solidFill"))
    if fill is None:
        fill = rpr.makeelement(_q(A, "solidFill"), {})
        rpr.insert(0, fill)
    _solid(fill, color)


def apply_theme(prs, name):
    """Restyle the master: theme fonts and colours, title/body sizes, spacing."""
    from lxml import etree
    from pptx.opc.constants import RELATIONSHIP_TYPE as RT

    t = THEMES.get(name) or THEMES["modern"]
    master = prs.slide_master
    part = master.part.part_related_by(RT.THEME)
    root = etree.fromstring(part.blob)
    for tag, face in (("majorFont", t["heading"]), ("minorFont", t["body"])):
        latin = root.find(f".//{_q(A, 'fontScheme')}/{_q(A, tag)}/{_q(A, 'latin')}")
        if latin is not None:
            latin.set("typeface", face)
    scheme = root.find(f".//{_q(A, 'clrScheme')}")
    if scheme is not None:
        for slot, key in (("dk1", "ink"), ("dk2", "muted"), ("lt2", "light"), ("accent1", "accent"), ("accent2", "muted"), ("hlink", "accent")):
            node = scheme.find(_q(A, slot))
            if node is not None:
                _solid(node, t[key])
    part._blob = etree.tostring(root, xml_declaration=True, encoding="UTF-8", standalone=True)

    styles = master._element.find(_q(P, "txStyles"))
    if styles is not None:
        title = styles.find(_q(P, "titleStyle"))
        if title is not None and title.find(_q(A, "lvl1pPr")) is not None:
            _lvl_style(title.find(_q(A, "lvl1pPr")), 34, t["ink"], bold=False, align="l")
        body = styles.find(_q(P, "bodyStyle"))
        if body is not None:
            for i, size in enumerate((20, 18, 16, 14, 14), start=1):
                lvl = body.find(_q(A, f"lvl{i}pPr"))
                if lvl is not None:
                    _lvl_style(lvl, size, t["ink"] if i == 1 else t["muted"], space_before=10 if i == 1 else 4)
    return t


def style_title_slide(slide, size):
    """Left-aligned title with the subtitle right below it, muted."""
    t = CURRENT
    title = slide.shapes.title
    subs = [ph for ph in body_placeholders(slide) if ph.text_frame.text.strip()]
    if title is not None:
        for para in title.text_frame.paragraphs:
            para.alignment = PP_ALIGN.LEFT
            for run in para.runs:
                run.font.size = Pt(size)
                run.font.color.rgb = RGBColor.from_string(t["ink"])
    for sub in subs[:1]:
        if title is not None and title.top is not None:
            sub.left, sub.width = title.left, title.width
            sub.top = title.top + title.height + Inches(0.05)
        for para in sub.text_frame.paragraphs:
            para.alignment = PP_ALIGN.LEFT
            for run in para.runs:
                run.font.size = Pt(20)
                run.font.color.rgb = RGBColor.from_string(t["muted"])


def style_table(table, t):
    """Light header band, hairline rules, no zebra stripes."""
    tbl = table._tbl
    pr = tbl.tblPr
    pr.set("bandRow", "0")
    pr.set("firstRow", "1")
    for r, row in enumerate(table.rows):
        for cell in row.cells:
            cell.fill.solid()
            cell.fill.fore_color.rgb = RGBColor.from_string(t["light"] if r == 0 else "FFFFFF")
            cell.margin_left = cell.margin_right = Inches(0.12)
            for p in cell.text_frame.paragraphs:
                for run in p.runs:
                    run.font.size = Pt(14)
                    run.font.bold = r == 0
                    run.font.color.rgb = RGBColor.from_string(t["ink"])
            tcpr = cell._tc.get_or_add_tcPr()
            for edge in ("lnL", "lnR", "lnT", "lnB"):
                for old in tcpr.findall(_q(A, edge)):
                    tcpr.remove(old)
                visible = edge == "lnB"
                ln = tcpr.makeelement(_q(A, edge), {"w": str(12700 if visible else 0)})
                if visible:
                    fill = ln.makeelement(_q(A, "solidFill"), {})
                    ln.append(fill)
                    _solid(fill, t["rule"])
                else:
                    ln.append(ln.makeelement(_q(A, "noFill"), {}))
                tcpr.insert(0, ln) if edge == "lnL" else tcpr.append(ln)



def parse_outline(src):
    """Markdown outline → deck spec."""
    spec = {"slides": []}
    cur = None
    lines = src.splitlines()
    i = 0
    seen_title = False

    def bullets_target():
        if cur.get("layout") == "two_column":
            return cur["right"] if cur.get("_split") else cur["left"]
        return cur.setdefault("bullets", [])

    while i < len(lines):
        raw = lines[i]
        s = raw.strip()
        i += 1
        if not s:
            continue
        if re.match(r"^# ", s):
            title = s[2:].strip()
            if not seen_title:
                seen_title = True
                spec["title"] = title
                cur = {"layout": "title", "title": title}
                spec["_title_slide"] = cur
            else:
                cur = {"layout": "section", "title": title}
                spec["slides"].append(cur)
            continue
        if re.match(r"^##+ ", s):
            title = s.lstrip("#").strip()
            if "|" in title:
                left, right = [t.strip() for t in title.split("|", 1)]
                cur = {"layout": "two_column", "title": f"{left} vs {right}" if left and right else title.replace("|", ""),
                       "left_title": left, "right_title": right, "left": [], "right": []}
            else:
                cur = {"layout": "bullets", "title": title}
            spec["slides"].append(cur)
            continue
        if cur is None:
            cur = {"layout": "bullets", "title": ""}
            spec["slides"].append(cur)
        if cur.get("layout") == "title" and not re.match(r"^([-*+]|\d+[.)]|>|!\[|\|)", s):
            cur["subtitle"] = (cur.get("subtitle", "") + " " + s).strip()
            spec["subtitle"] = cur["subtitle"]
            continue
        if s == "---" and cur.get("layout") == "two_column":
            cur["_split"] = True
            continue
        if s.startswith(">"):
            cur["notes"] = (cur.get("notes", "") + "\n" + s.lstrip("> ").strip()).strip()
            continue
        m = re.match(r"^!\[([^\]]*)\]\(([^)]+)\)$", s)
        if m:
            cur["layout"] = "image"
            cur["image"] = m.group(2)
            cur["caption"] = m.group(1)
            continue
        if s.startswith("|"):
            rows = [[c.strip() for c in s.strip("|").split("|")]]
            while i < len(lines) and lines[i].strip().startswith("|"):
                cells = [c.strip() for c in lines[i].strip().strip("|").split("|")]
                if not all(re.match(r"^:?-{2,}:?$", c) for c in cells if c):
                    rows.append(cells)
                i += 1
            cur["layout"] = "table"
            cur["header"], cur["rows"] = rows[0], rows[1:]
            continue
        bm = re.match(r"^(\s*)([-*+]|\d+[.)])\s+(.*)$", raw)
        if bm:
            depth = len(bm.group(1).replace("\t", "  ")) // 2
            target = bullets_target()
            node = target
            for _ in range(depth):
                if node and isinstance(node[-1], list):
                    node = node[-1]
                else:
                    node.append([])
                    node = node[-1]
            node.append(bm.group(3).strip())
            continue
        bullets_target().append(s)
    title_slide = spec.pop("_title_slide", None)
    for sl in spec["slides"]:
        sl.pop("_split", None)
    if title_slide is None:
        spec["no_title_slide"] = True
    return spec


def widen(prs):
    """Switch the default 4:3 template to 16:9 and rescale its layouts."""
    old_w = prs.slide_width
    if old_w == WIDE[0]:
        return
    ratio = WIDE[0] / old_w
    prs.slide_width, prs.slide_height = WIDE
    for master in prs.slide_masters:
        for shapes in [master.shapes] + [layout.shapes for layout in master.slide_layouts]:
            for sh in shapes:
                # Only shapes with their own frame: the rest inherit the
                # (already widened) master. Set all four values, or
                # python-pptx writes a zero top/height.
                if getattr(sh._element, "xfrm", None) is None:
                    continue
                frame = (sh.left, sh.top, sh.width, sh.height)
                if None in frame:
                    continue
                left, top, width, height = frame
                sh.left, sh.top, sh.width, sh.height = Emu(int(left * ratio)), top, Emu(int(width * ratio)), height


def layout_by_name(prs, *names):
    layouts = list(prs.slide_layouts)
    for name in names:
        for layout in layouts:
            if layout.name.lower() == name.lower():
                return layout
    return layouts[min(1, len(layouts) - 1)]


def placeholder(slide, *types):
    for ph in slide.placeholders:
        t = ph.placeholder_format.type
        if t is not None and any(str(t).split()[0].upper().endswith(x) for x in types):
            return ph
    return None


def body_placeholders(slide):
    out = []
    for ph in slide.placeholders:
        idx = ph.placeholder_format.idx
        if idx not in (0,) and ph.has_text_frame and not str(ph.placeholder_format.type).startswith(("TITLE", "CENTER_TITLE", "DATE", "FOOTER", "SLIDE_NUMBER")):
            out.append(ph)
    return sorted(out, key=lambda p: (p.left or 0, p.top or 0))


def fill_bullets(frame, items, level=0, first=True):
    for item in items:
        if isinstance(item, list):
            first = fill_bullets(frame, item, level + 1, first)
            continue
        p = frame.paragraphs[0] if first else frame.add_paragraph()
        first = False
        p.text = str(item)
        p.level = min(level, 4)
    return first


def set_title(slide, text):
    if slide.shapes.title is not None:
        slide.shapes.title.text = text or ""


def remove_empty_placeholders(slide):
    for ph in list(slide.placeholders):
        if ph.has_text_frame and not ph.text_frame.text.strip():
            ph._element.getparent().remove(ph._element)


def add_slide(prs, s, base_dir):
    kind = s.get("layout", "bullets")
    if kind in ("title", "closing"):
        slide = prs.slides.add_slide(layout_by_name(prs, "Title Slide"))
        set_title(slide, s.get("title"))
        sub = body_placeholders(slide)
        if sub and s.get("subtitle"):
            sub[0].text = s["subtitle"]
        title = slide.shapes.title
        if title is not None and title.top is not None:
            bar = slide.shapes.add_shape(MSO_SHAPE.RECTANGLE, title.left + Inches(0.1), title.top - Inches(0.25), Inches(0.9), Pt(4))
            bar.fill.solid()
            bar.fill.fore_color.rgb = RGBColor.from_string(CURRENT["accent"])
            bar.line.fill.background()
            bar.shadow.inherit = False
        style_title_slide(slide, 44)
    elif kind == "section":
        slide = prs.slides.add_slide(layout_by_name(prs, "Section Header", "Title Only"))
        set_title(slide, s.get("title"))
        sub = body_placeholders(slide)
        if sub and s.get("subtitle"):
            sub[0].text = s["subtitle"]
        style_title_slide(slide, 36)
    elif kind == "two_column":
        slide = prs.slides.add_slide(layout_by_name(prs, "Two Content", "Comparison"))
        set_title(slide, s.get("title"))
        bodies = body_placeholders(slide)
        cols = [(s.get("left_title"), s.get("left", [])), (s.get("right_title"), s.get("right", []))]
        for ph, (head, items) in zip(bodies, cols):
            content = ([f"{head}"] + [items] if head else items) if items or head else []
            fill_bullets(ph.text_frame, content)
            if head:
                ph.text_frame.paragraphs[0].runs[0].font.bold = True
    elif kind == "image":
        slide = prs.slides.add_slide(layout_by_name(prs, "Title Only"))
        set_title(slide, s.get("title"))
        path = s.get("image", "")
        if not os.path.isabs(path):
            path = os.path.join(base_dir, path)
        top = Inches(1.6)
        max_w, max_h = prs.slide_width - Inches(1.5), prs.slide_height - top - Inches(0.9)
        pic = slide.shapes.add_picture(path, 0, top)
        scale = min(max_w / pic.width, max_h / pic.height, 1.0)
        pic.width, pic.height = int(pic.width * scale), int(pic.height * scale)
        pic.left = int((prs.slide_width - pic.width) / 2)
        if s.get("caption"):
            box = slide.shapes.add_textbox(Inches(0.75), top + pic.height + Inches(0.1), prs.slide_width - Inches(1.5), Inches(0.5))
            p = box.text_frame.paragraphs[0]
            p.text = s["caption"]
            p.runs[0].font.size = Pt(14)
            p.runs[0].font.color.rgb = RGBColor.from_string(CURRENT["muted"])
            p.alignment = 2
    elif kind == "table":
        slide = prs.slides.add_slide(layout_by_name(prs, "Title Only"))
        set_title(slide, s.get("title"))
        header, rows = s.get("header", []), s.get("rows", [])
        cols = max([len(header)] + [len(r) for r in rows] + [1])
        n = len(rows) + (1 if header else 0)
        left, top = Inches(0.75), Inches(1.7)
        width = prs.slide_width - Inches(1.5)
        shape = slide.shapes.add_table(n, cols, left, top, width, Inches(0.45) * n)
        table = shape.table
        r0 = 0
        if header:
            for j in range(cols):
                table.cell(0, j).text = str(header[j]) if j < len(header) else ""
            r0 = 1
        for i, row in enumerate(rows):
            for j in range(cols):
                table.cell(r0 + i, j).text = str(row[j]) if j < len(row) else ""
        style_table(table, CURRENT)
    elif kind == "quote":
        slide = prs.slides.add_slide(layout_by_name(prs, "Title Only", "Blank"))
        set_title(slide, s.get("title", ""))
        box = slide.shapes.add_textbox(Inches(1.5), Inches(2.4), prs.slide_width - Inches(3), Inches(2.5))
        tf = box.text_frame
        tf.word_wrap = True
        tf.paragraphs[0].text = f"\u201c{s.get('text', '')}\u201d"
        tf.paragraphs[0].runs[0].font.size = Pt(34)
        tf.paragraphs[0].runs[0].font.name = CURRENT["heading"]
        tf.paragraphs[0].runs[0].font.color.rgb = RGBColor.from_string(CURRENT["ink"])
        if s.get("by"):
            p = tf.add_paragraph()
            p.text = f"\u2014 {s['by']}"
            p.runs[0].font.size = Pt(18)
            p.runs[0].font.color.rgb = RGBColor.from_string(CURRENT["muted"])
    else:
        slide = prs.slides.add_slide(layout_by_name(prs, "Title and Content"))
        set_title(slide, s.get("title"))
        bodies = body_placeholders(slide)
        if bodies:
            fill_bullets(bodies[0].text_frame, s.get("bullets", []))
    if s.get("notes"):
        slide.notes_slide.notes_text_frame.text = s["notes"]
    remove_empty_placeholders(slide)
    pin_placeholders(slide)
    return slide


def pin_placeholders(slide):
    """Write inherited placeholder frames onto the slide, so viewers that do
    not resolve layout inheritance (previewers, some web apps) lay it out
    the same way PowerPoint does."""
    for ph in slide.placeholders:
        frame = (ph.left, ph.top, ph.width, ph.height)
        if None not in frame:
            ph.left, ph.top, ph.width, ph.height = frame


def read_input(path):
    """The input file's text; `-` reads standard input."""
    if path == "-":
        return sys.stdin.read()
    with open(path, encoding="utf-8") as f:
        return f.read()


def is_json(path, src):
    return path.lower().endswith(".json") or (path == "-" and src.lstrip().startswith("{"))


def load_spec(path):
    src = read_input(path)
    return json.loads(src) if is_json(path, src) else parse_outline(src)


def open_deck(template):
    prs = Presentation(template) if template else Presentation()
    if template:
        # Keep the template's masters and layouts, drop its sample slides.
        sld_ids = prs.slides._sldIdLst
        for sld in list(sld_ids):
            prs.part.drop_rel(sld.rId)
            sld_ids.remove(sld)
    return prs


def cmd_create(a):
    spec = load_spec(a.input)
    prs = open_deck(a.template)
    if spec.get("widescreen", True) and not a.template:
        widen(prs)
    global CURRENT
    theme = spec.get("theme") or a.theme
    if not a.template or a.theme:
        CURRENT = apply_theme(prs, theme or "modern")
    base_dir = os.path.dirname(os.path.abspath(a.input))
    if spec.get("title") and not spec.get("no_title_slide"):
        add_slide(prs, {"layout": "title", "title": spec["title"], "subtitle": spec.get("subtitle")}, base_dir)
    for s in spec.get("slides", []):
        add_slide(prs, s, base_dir)
    if spec.get("title"):
        prs.core_properties.title = spec["title"]
    prs.save(a.output)
    print(f"wrote {a.output} ({len(prs.slides)} slides)")


def shape_texts(slide):
    out = []
    for i, sh in enumerate(slide.shapes):
        kind = "picture" if sh.shape_type == 13 else ("table" if sh.has_table else "text" if sh.has_text_frame else str(sh.shape_type))
        text = ""
        if sh.has_text_frame:
            text = sh.text_frame.text
        elif sh.has_table:
            text = "\n".join(" | ".join(c.text for c in r.cells) for r in sh.table.rows)
        out.append({"shape": i, "name": sh.name, "kind": kind, "text": text})
    return out


def cmd_dump(a):
    prs = Presentation(a.file)
    slides = []
    for n, slide in enumerate(prs.slides, 1):
        notes = slide.notes_slide.notes_text_frame.text if slide.has_notes_slide else ""
        slides.append({"slide": n, "layout": slide.slide_layout.name, "shapes": shape_texts(slide), "notes": notes})
    if a.json:
        print(json.dumps({"width_in": prs.slide_width / 914400, "height_in": prs.slide_height / 914400, "slides": slides}, ensure_ascii=False, indent=1))
        return
    for s in slides:
        print(f"--- slide {s['slide']} ({s['layout']})")
        for sh in s["shapes"]:
            text = sh["text"].replace("\n", " / ")
            print(f"  [{sh['shape']}] {sh['kind']}: {text}")
        if s["notes"]:
            print(f"  notes: {s['notes']}")


def get_slide(prs, n):
    if not 1 <= n <= len(prs.slides):
        raise SystemExit(f"slide {n} out of range (1..{len(prs.slides)})")
    return prs.slides[n - 1]


def set_frame_text(frame, text):
    """Replace a text frame's text, keeping the first run's formatting per line."""
    lines = text.split("\n")
    paras = frame.paragraphs
    for i, line in enumerate(lines):
        if i < len(paras):
            p = paras[i]
            runs = p.runs
            if runs:
                runs[0].text = line
                for r in runs[1:]:
                    r._r.getparent().remove(r._r)
            else:
                p.text = line
        else:
            src = paras[-1]._p
            new = copy.deepcopy(src)
            src.addnext(new)
            paras = frame.paragraphs
            p = paras[i]
            runs = p.runs
            if runs:
                runs[0].text = line
                for r in runs[1:]:
                    r._r.getparent().remove(r._r)
            else:
                p.text = line
    for p in frame.paragraphs[len(lines):]:
        p._p.getparent().remove(p._p)


def cmd_set_title(a):
    prs = Presentation(a.file)
    slide = get_slide(prs, a.slide)
    if slide.shapes.title is None:
        raise SystemExit("that slide has no title placeholder; use set-text")
    set_frame_text(slide.shapes.title.text_frame, a.text)
    prs.save(a.output or a.file)
    print("ok")


def cmd_set_text(a):
    prs = Presentation(a.file)
    slide = get_slide(prs, a.slide)
    shapes = list(slide.shapes)
    if not 0 <= a.shape < len(shapes) or not shapes[a.shape].has_text_frame:
        raise SystemExit(f"shape {a.shape} is not a text shape on slide {a.slide} (see `dump`)")
    set_frame_text(shapes[a.shape].text_frame, a.text.replace("\\n", "\n"))
    prs.save(a.output or a.file)
    print("ok")


def cmd_replace(a):
    prs = Presentation(a.file)
    n = 0
    for slide in prs.slides:
        frames = []
        for sh in slide.shapes:
            if sh.has_text_frame:
                frames.append(sh.text_frame)
            elif sh.has_table:
                frames += [c.text_frame for r in sh.table.rows for c in r.cells]
        for tf in frames:
            for p in tf.paragraphs:
                for r in p.runs:
                    if a.old in r.text:
                        n += r.text.count(a.old)
                        r.text = r.text.replace(a.old, a.new)
                if a.old in "".join(r.text for r in p.runs) and p.runs:
                    text = "".join(r.text for r in p.runs)
                    n += text.count(a.old)
                    p.runs[0].text = text.replace(a.old, a.new)
                    for r in p.runs[1:]:
                        r.text = ""
    prs.save(a.output or a.file)
    print(f"replaced {n} occurrence(s)")


def cmd_add(a):
    prs = Presentation(a.file)
    spec = load_spec(a.input)
    base_dir = os.path.dirname(os.path.abspath(a.input))
    slides = spec.get("slides", [])
    for s in slides:
        add_slide(prs, s, base_dir)
    prs.save(a.output or a.file)
    print(f"added {len(slides)} slide(s)")


def cmd_delete(a):
    prs = Presentation(a.file)
    get_slide(prs, a.slide)
    sld = prs.slides._sldIdLst[a.slide - 1]
    prs.part.drop_rel(sld.rId)
    prs.slides._sldIdLst.remove(sld)
    prs.save(a.output or a.file)
    print(f"deleted slide {a.slide}")


def cmd_notes(a):
    prs = Presentation(a.file)
    get_slide(prs, a.slide).notes_slide.notes_text_frame.text = a.text
    prs.save(a.output or a.file)
    print("ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    def cmd(name, fn, *args, out=True, **kw):
        p = sub.add_parser(name, **kw)
        for arg, typ in args:
            p.add_argument(arg, type=typ)
        if out:
            p.add_argument("-o", "--output")
        p.set_defaults(fn=fn)
        return p

    c = cmd("create", cmd_create, ("input", str), ("output", str), out=False, help="outline (.md/.json) → .pptx")
    c.add_argument("--template")
    c.add_argument("--theme", choices=sorted(THEMES), help="look of the deck (default: modern; templates keep their own unless set)")
    d = cmd("dump", cmd_dump, ("file", str), out=False, help="print every slide's text")
    d.add_argument("--json", action="store_true")
    cmd("set-title", cmd_set_title, ("file", str), ("slide", int), ("text", str))
    cmd("set-text", cmd_set_text, ("file", str), ("slide", int), ("shape", int), ("text", str))
    cmd("replace", cmd_replace, ("file", str), ("old", str), ("new", str))
    cmd("add", cmd_add, ("file", str), ("input", str))
    cmd("delete", cmd_delete, ("file", str), ("slide", int))
    cmd("notes", cmd_notes, ("file", str), ("slide", int), ("text", str))
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
