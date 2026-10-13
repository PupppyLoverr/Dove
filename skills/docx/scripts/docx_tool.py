#!/usr/bin/env python3
# /// script
# requires-python = ">=3.9"
# dependencies = ["python-docx>=1.1"]
# ///
"""Create, inspect and edit .docx files. See ../SKILL.md."""
import argparse
import json
import os
import re
import sys

try:
    import docx
    from docx.enum.section import WD_ORIENT
    from docx.enum.table import WD_TABLE_ALIGNMENT
    from docx.enum.text import WD_ALIGN_PARAGRAPH, WD_BREAK
    from docx.oxml import OxmlElement
    from docx.oxml.ns import qn
    from docx.shared import Inches, Pt, RGBColor
except ImportError:
    sys.exit("python-docx is missing: run with `uv run` or `python3 -m pip install --user python-docx`")

ALIGN = {
    "left": WD_ALIGN_PARAGRAPH.LEFT,
    "center": WD_ALIGN_PARAGRAPH.CENTER,
    "right": WD_ALIGN_PARAGRAPH.RIGHT,
    "justify": WD_ALIGN_PARAGRAPH.JUSTIFY,
}

INLINE = re.compile(r"(\*\*[^*]+\*\*|\*[^*]+\*|`[^`]+`)")


def inline_runs(text):
    runs = []
    for part in INLINE.split(text):
        if not part:
            continue
        if part.startswith("**") and part.endswith("**"):
            runs.append({"text": part[2:-2], "bold": True})
        elif part.startswith("`") and part.endswith("`"):
            runs.append({"text": part[1:-1], "code": True})
        elif part.startswith("*") and part.endswith("*") and len(part) > 2:
            runs.append({"text": part[1:-1], "italic": True})
        else:
            runs.append({"text": part})
    return runs


def markdown_blocks(src):
    """A small Markdown subset → JSON blocks."""
    blocks, para, lines = [], [], src.splitlines()
    i = 0

    def flush():
        if para:
            blocks.append({"type": "paragraph", "runs": inline_runs(" ".join(para))})
            para.clear()

    while i < len(lines):
        line = lines[i]
        s = line.strip()
        if not s:
            flush()
            i += 1
            continue
        if s == "\\pagebreak":
            flush()
            blocks.append({"type": "page_break"})
        elif re.match(r"^#{1,6} ", s):
            flush()
            level = len(s) - len(s.lstrip("#"))
            blocks.append({"type": "heading", "text": s[level:].strip(), "level": min(level, 4)})
        elif re.match(r"^!\[[^\]]*\]\([^)]+\)$", s):
            flush()
            m = re.match(r"^!\[([^\]]*)\]\(([^)]+)\)$", s)
            blocks.append({"type": "image", "path": m.group(2), "caption": m.group(1) or None})
        elif s.startswith(">"):
            flush()
            blocks.append({"type": "quote", "text": s.lstrip("> ").strip()})
        elif s.startswith("|"):
            flush()
            rows = []
            while i < len(lines) and lines[i].strip().startswith("|"):
                cells = [c.strip() for c in lines[i].strip().strip("|").split("|")]
                if not all(re.match(r"^:?-{2,}:?$", c) for c in cells if c):
                    rows.append(cells)
                i += 1
            if rows:
                blocks.append({"type": "table", "header": rows[0], "rows": rows[1:]})
            continue
        elif re.match(r"^\s*([-*+]|\d+[.)])\s+", line):
            flush()
            numbered = bool(re.match(r"^\s*\d+[.)]\s+", line))
            items = []
            while i < len(lines) and re.match(r"^\s*([-*+]|\d+[.)])\s+", lines[i]):
                raw = lines[i]
                depth = (len(raw) - len(raw.lstrip())) // 2
                text = re.sub(r"^\s*([-*+]|\d+[.)])\s+", "", raw)
                items.append((depth, text))
                i += 1
            blocks.append({"type": "numbered" if numbered else "bullets", "items": nest(items)})
            continue
        else:
            para.append(s)
        i += 1
    flush()
    return blocks


def nest(items):
    """[(depth, text)] → nested lists (a list item may be a sub-list)."""
    root = []
    stack = [root]
    for depth, text in items:
        depth = min(depth, len(stack))
        while len(stack) - 1 > depth:
            stack.pop()
        if len(stack) - 1 < depth:
            child = []
            stack[-1].append(child)
            stack.append(child)
        stack[-1].append(text)
    return root


def add_runs(paragraph, runs):
    for r in runs:
        run = paragraph.add_run(r.get("text", ""))
        run.bold = r.get("bold") or None
        run.italic = r.get("italic") or None
        run.underline = r.get("underline") or None
        if r.get("code"):
            run.font.name = "Menlo"
            run.font.size = Pt(9.5)
        if r.get("color"):
            run.font.color.rgb = RGBColor.from_string(r["color"].lstrip("#"))
        if r.get("size"):
            run.font.size = Pt(r["size"])


def set_cell_background(cell, hex_fill):
    tc_pr = cell._tc.get_or_add_tcPr()
    shd = OxmlElement("w:shd")
    shd.set(qn("w:val"), "clear")
    shd.set(qn("w:color"), "auto")
    shd.set(qn("w:fill"), hex_fill)
    tc_pr.append(shd)


def add_page_number(paragraph):
    run = paragraph.add_run()
    for kind, text in (("begin", None), (None, "PAGE"), ("end", None)):
        if kind:
            el = OxmlElement("w:fldChar")
            el.set(qn("w:fldCharType"), kind)
        else:
            el = OxmlElement("w:instrText")
            el.set(qn("xml:space"), "preserve")
            el.text = text
        run._r.append(el)


def list_style(doc, numbered, depth):
    base = "List Number" if numbered else "List Bullet"
    name = base if depth == 0 else f"{base} {min(depth + 1, 3)}"
    try:
        doc.styles[name]
        return name
    except KeyError:
        return base


def add_list(doc, items, numbered, depth=0, base_dir="."):
    for item in items:
        if isinstance(item, list):
            add_list(doc, item, numbered, depth + 1, base_dir)
        else:
            p = doc.add_paragraph(style=list_style(doc, numbered, depth))
            add_runs(p, inline_runs(str(item)))


def add_block(doc, b, base_dir):
    kind = b.get("type", "paragraph")
    if kind == "heading":
        doc.add_heading(b.get("text", ""), level=int(b.get("level", 1)))
    elif kind == "paragraph":
        p = doc.add_paragraph(style=b.get("style"))
        add_runs(p, b.get("runs") or inline_runs(b.get("text", "")))
        if b.get("align") in ALIGN:
            p.alignment = ALIGN[b["align"]]
    elif kind in ("bullets", "numbered"):
        add_list(doc, b.get("items", []), kind == "numbered", 0, base_dir)
    elif kind == "quote":
        style = "Intense Quote" if "Intense Quote" in [s.name for s in doc.styles] else None
        p = doc.add_paragraph(style=style)
        add_runs(p, inline_runs(b.get("text", "")))
    elif kind == "table":
        header = b.get("header") or []
        rows = b.get("rows") or []
        cols = max([len(header)] + [len(r) for r in rows] + [1])
        table = doc.add_table(rows=0, cols=cols)
        table.style = b.get("style", "Table Grid")
        table.alignment = WD_TABLE_ALIGNMENT.CENTER
        if header:
            cells = table.add_row().cells
            for j in range(cols):
                cells[j].text = ""
                run = cells[j].paragraphs[0].add_run(str(header[j]) if j < len(header) else "")
                run.bold = True
                set_cell_background(cells[j], b.get("header_fill", "F2F2F2"))
            mark_header_row(table.rows[0])
        for r in rows:
            cells = table.add_row().cells
            for j in range(cols):
                cells[j].text = ""
                add_runs(cells[j].paragraphs[0], inline_runs(str(r[j]) if j < len(r) else ""))
        for j, w in enumerate(b.get("widths") or []):
            for row in table.rows:
                if j < len(row.cells):
                    row.cells[j].width = Inches(w)
        doc.add_paragraph()
    elif kind == "image":
        path = b.get("path", "")
        if not os.path.isabs(path):
            path = os.path.join(base_dir, path)
        doc.add_picture(path, width=Inches(float(b.get("width", 6.0))))
        doc.paragraphs[-1].alignment = WD_ALIGN_PARAGRAPH.CENTER
        if b.get("caption"):
            style = "Caption" if "Caption" in [s.name for s in doc.styles] else None
            cap = doc.add_paragraph(b["caption"], style=style)
            cap.alignment = WD_ALIGN_PARAGRAPH.CENTER
    elif kind == "page_break":
        doc.add_paragraph().add_run().add_break(WD_BREAK.PAGE)
    else:
        raise SystemExit(f"unknown block type: {kind}")


def mark_header_row(row):
    tr_pr = row._tr.get_or_add_trPr()
    el = OxmlElement("w:tblHeader")
    el.set(qn("w:val"), "true")
    tr_pr.append(el)


def load_spec(path):
    with open(path, encoding="utf-8") as f:
        src = f.read()
    if path.lower().endswith(".json"):
        spec = json.loads(src)
    else:
        spec = {"blocks": markdown_blocks(src)}
    return spec


def new_document(template=None):
    if template:
        doc = docx.Document(template)
        body = doc.element.body
        for child in list(body):
            if child.tag != qn("w:sectPr"):
                body.remove(child)
        return doc
    return docx.Document()


def cmd_create(a):
    spec = load_spec(a.input)
    if a.title:
        spec["title"] = a.title
    doc = new_document(a.template)
    normal = doc.styles["Normal"]
    normal.font.name = spec.get("font", "Calibri")
    normal.font.size = Pt(spec.get("size", 11))
    if spec.get("landscape"):
        sec = doc.sections[0]
        sec.orientation = WD_ORIENT.LANDSCAPE
        sec.page_width, sec.page_height = sec.page_height, sec.page_width
    if spec.get("author"):
        doc.core_properties.author = spec["author"]
    if spec.get("title"):
        doc.core_properties.title = spec["title"]
        doc.add_heading(spec["title"], level=0)
        if spec.get("subtitle"):
            style = "Subtitle" if "Subtitle" in [s.name for s in doc.styles] else None
            doc.add_paragraph(spec["subtitle"], style=style)
    base_dir = os.path.dirname(os.path.abspath(a.input))
    for b in spec.get("blocks", []):
        add_block(doc, b, base_dir)
    sec = doc.sections[0]
    if spec.get("header"):
        sec.header.paragraphs[0].text = spec["header"]
    if spec.get("footer"):
        p = sec.footer.paragraphs[0]
        p.alignment = WD_ALIGN_PARAGRAPH.CENTER
        if spec["footer"] == "page":
            add_page_number(p)
        else:
            p.text = spec["footer"]
    doc.save(a.output)
    print(f"wrote {a.output}")


def iter_paragraphs(doc):
    for p in doc.paragraphs:
        yield p


def cmd_dump(a):
    doc = docx.Document(a.file)
    paras = [{"index": i, "style": p.style.name if p.style is not None else "", "text": p.text} for i, p in enumerate(doc.paragraphs)]
    tables = [[[c.text for c in row.cells] for row in t.rows] for t in doc.tables]
    if a.json:
        print(json.dumps({"paragraphs": paras, "tables": tables}, ensure_ascii=False, indent=1))
        return
    for p in paras:
        if p["text"].strip():
            print(f"[{p['index']}] ({p['style']}) {p['text']}")
    for ti, t in enumerate(tables):
        print(f"\n# table {ti} ({len(t)} rows)")
        for row in t:
            print(" | ".join(row))


def replace_in_paragraph(p, old, new):
    if old not in p.text:
        return 0
    count = 0
    for run in p.runs:
        if old in run.text:
            count += run.text.count(old)
            run.text = run.text.replace(old, new)
    if old in p.text:
        # The match spans runs: move the paragraph's text into its first run.
        text = p.text
        count += text.count(old)
        for i, run in enumerate(p.runs):
            run.text = text.replace(old, new) if i == 0 else ""
    return count


def all_paragraphs(doc):
    yield from doc.paragraphs
    for t in doc.tables:
        for row in t.rows:
            for cell in row.cells:
                yield from cell.paragraphs
    for sec in doc.sections:
        for part in (sec.header, sec.footer):
            yield from part.paragraphs


def cmd_replace(a):
    doc = docx.Document(a.file)
    n = sum(replace_in_paragraph(p, a.old, a.new) for p in all_paragraphs(doc))
    doc.save(a.output or a.file)
    print(f"replaced {n} occurrence(s)")


def set_paragraph_text(p, text):
    runs = p.runs
    if not runs:
        p.add_run(text)
        return
    runs[0].text = text
    for r in runs[1:]:
        r.text = ""


def cmd_set(a):
    doc = docx.Document(a.file)
    if not 0 <= a.index < len(doc.paragraphs):
        raise SystemExit(f"paragraph {a.index} out of range (0..{len(doc.paragraphs) - 1})")
    set_paragraph_text(doc.paragraphs[a.index], a.text)
    doc.save(a.output or a.file)
    print(f"set paragraph {a.index}")


def cmd_delete(a):
    doc = docx.Document(a.file)
    p = doc.paragraphs[a.index]
    p._element.getparent().remove(p._element)
    doc.save(a.output or a.file)
    print(f"deleted paragraph {a.index}")


def cmd_append(a):
    doc = docx.Document(a.file)
    spec = load_spec(a.input)
    base_dir = os.path.dirname(os.path.abspath(a.input))
    for b in spec.get("blocks", []):
        add_block(doc, b, base_dir)
    doc.save(a.output or a.file)
    print(f"appended {len(spec.get('blocks', []))} block(s)")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("create", help="Markdown or JSON spec → .docx")
    c.add_argument("input")
    c.add_argument("output")
    c.add_argument("--title")
    c.add_argument("--template")
    c.set_defaults(fn=cmd_create)
    d = sub.add_parser("dump", help="print paragraphs and tables")
    d.add_argument("file")
    d.add_argument("--json", action="store_true")
    d.set_defaults(fn=cmd_dump)
    r = sub.add_parser("replace", help="find/replace text keeping formatting")
    r.add_argument("file")
    r.add_argument("old")
    r.add_argument("new")
    r.add_argument("-o", "--output")
    r.set_defaults(fn=cmd_replace)
    s = sub.add_parser("set", help="replace one paragraph's text")
    s.add_argument("file")
    s.add_argument("index", type=int)
    s.add_argument("text")
    s.add_argument("-o", "--output")
    s.set_defaults(fn=cmd_set)
    x = sub.add_parser("delete", help="remove one paragraph")
    x.add_argument("file")
    x.add_argument("index", type=int)
    x.add_argument("-o", "--output")
    x.set_defaults(fn=cmd_delete)
    p = sub.add_parser("append", help="append Markdown/JSON blocks")
    p.add_argument("file")
    p.add_argument("input")
    p.add_argument("-o", "--output")
    p.set_defaults(fn=cmd_append)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
