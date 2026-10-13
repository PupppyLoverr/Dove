#!/usr/bin/env python3
# /// script
# requires-python = ">=3.9"
# dependencies = ["reportlab>=4.0", "pypdf>=4.0"]
# ///
"""Create, inspect, combine and fill PDFs. See ../SKILL.md."""
import argparse
import json
import os
import re
import sys
from io import BytesIO

try:
    from pypdf import PdfReader, PdfWriter
    from pypdf.generic import NameObject
    from reportlab.lib import colors
    from reportlab.lib.enums import TA_CENTER
    from reportlab.lib.pagesizes import A4, LETTER, landscape
    from reportlab.lib.styles import ParagraphStyle, getSampleStyleSheet
    from reportlab.lib.units import inch
    from reportlab.pdfgen import canvas
    from reportlab.platypus import (Image, ListFlowable, ListItem, PageBreak, Paragraph, SimpleDocTemplate,
                                    Spacer, Table, TableStyle)
except ImportError:
    sys.exit("reportlab/pypdf are missing: run with `uv run` or `python3 -m pip install --user reportlab pypdf`")

INLINE = [
    (re.compile(r"\*\*([^*]+)\*\*"), r"<b>\1</b>"),
    (re.compile(r"(?<![*\w])\*([^*]+)\*(?!\*)"), r"<i>\1</i>"),
    (re.compile(r"`([^`]+)`"), r'<font face="Courier">\1</font>'),
    (re.compile(r"\[([^\]]+)\]\(([^)]+)\)"), r'<a href="\2" color="#333333"><u>\1</u></a>'),
]


def md_inline(text):
    text = text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    for pat, rep in INLINE:
        text = pat.sub(rep, text)
    return text


def markdown_blocks(src):
    blocks, para, lines = [], [], src.splitlines()
    i = 0

    def flush():
        if para:
            blocks.append({"type": "paragraph", "text": md_inline(" ".join(para))})
            para.clear()

    while i < len(lines):
        line, s = lines[i], lines[i].strip()
        if not s:
            flush()
        elif s == "\\pagebreak":
            flush()
            blocks.append({"type": "page_break"})
        elif re.match(r"^#{1,6} ", s):
            flush()
            level = len(s) - len(s.lstrip("#"))
            blocks.append({"type": "heading", "text": md_inline(s[level:].strip()), "level": min(level, 3)})
        elif re.match(r"^!\[[^\]]*\]\([^)]+\)$", s):
            flush()
            m = re.match(r"^!\[([^\]]*)\]\(([^)]+)\)$", s)
            blocks.append({"type": "image", "path": m.group(2), "caption": m.group(1) or None})
        elif s.startswith(">"):
            flush()
            blocks.append({"type": "quote", "text": md_inline(s.lstrip("> ").strip())})
        elif s.startswith("|"):
            flush()
            rows = []
            while i < len(lines) and lines[i].strip().startswith("|"):
                cells = [c.strip() for c in lines[i].strip().strip("|").split("|")]
                if not all(re.match(r"^:?-{2,}:?$", c) for c in cells if c):
                    rows.append([md_inline(c) for c in cells])
                i += 1
            if rows:
                blocks.append({"type": "table", "header": rows[0], "rows": rows[1:]})
            continue
        elif re.match(r"^\s*([-*+]|\d+[.)])\s+", line):
            flush()
            numbered = bool(re.match(r"^\s*\d+[.)]\s+", line))
            items = []
            while i < len(lines) and re.match(r"^\s*([-*+]|\d+[.)])\s+", lines[i]):
                items.append(md_inline(re.sub(r"^\s*([-*+]|\d+[.)])\s+", "", lines[i])))
                i += 1
            blocks.append({"type": "numbered" if numbered else "bullets", "items": items})
            continue
        else:
            para.append(s)
        i += 1
    flush()
    return blocks


def styles():
    ss = getSampleStyleSheet()
    base = ParagraphStyle("Body", parent=ss["BodyText"], fontName="Helvetica", fontSize=10.5, leading=15, spaceAfter=6)
    return {
        "body": base,
        "title": ParagraphStyle("DocTitle", parent=ss["Title"], fontName="Helvetica-Bold", fontSize=24, leading=29, spaceAfter=14, alignment=0),
        1: ParagraphStyle("H1", parent=ss["Heading1"], fontName="Helvetica-Bold", fontSize=17, leading=21, spaceBefore=12, spaceAfter=6),
        2: ParagraphStyle("H2", parent=ss["Heading2"], fontName="Helvetica-Bold", fontSize=13.5, leading=17, spaceBefore=10, spaceAfter=4),
        3: ParagraphStyle("H3", parent=ss["Heading3"], fontName="Helvetica-Bold", fontSize=11.5, leading=15, spaceBefore=8, spaceAfter=3),
        "quote": ParagraphStyle("Quote", parent=base, leftIndent=18, textColor=colors.HexColor("#444444"), fontName="Helvetica-Oblique"),
        "caption": ParagraphStyle("Caption", parent=base, fontSize=9, textColor=colors.HexColor("#555555"), alignment=TA_CENTER),
        "cell": ParagraphStyle("Cell", parent=base, fontSize=9.5, leading=12, spaceAfter=0),
        "head": ParagraphStyle("Head", parent=base, fontSize=9.5, leading=12, spaceAfter=0, fontName="Helvetica-Bold"),
    }


def flowables(blocks, st, base_dir, frame_width):
    out = []
    for b in blocks:
        kind = b.get("type", "paragraph")
        if kind == "heading":
            out.append(Paragraph(b.get("text", ""), st.get(int(b.get("level", 1)), st[3])))
        elif kind == "paragraph":
            out.append(Paragraph(b.get("text", ""), st["body"]))
        elif kind == "quote":
            out.append(Paragraph(b.get("text", ""), st["quote"]))
        elif kind in ("bullets", "numbered"):
            items = [ListItem(Paragraph(str(t), st["body"]), leftIndent=14) for t in b.get("items", [])]
            out.append(ListFlowable(items, bulletType="1" if kind == "numbered" else "bullet", start="1" if kind == "numbered" else None,
                                    leftIndent=14, bulletFontSize=8 if kind == "bullets" else 10))
        elif kind == "table":
            header, rows = b.get("header") or [], b.get("rows") or []
            cols = max([len(header)] + [len(r) for r in rows] + [1])
            data = []
            if header:
                data.append([Paragraph(str(h), st["head"]) for h in header] + [""] * (cols - len(header)))
            for r in rows:
                data.append([Paragraph(str(c), st["cell"]) for c in r] + [""] * (cols - len(r)))
            widths = [w * inch for w in b["widths"]] if b.get("widths") else [frame_width / cols] * cols
            t = Table(data, colWidths=widths, repeatRows=1 if header else 0)
            style = [
                ("VALIGN", (0, 0), (-1, -1), "TOP"),
                ("LINEBELOW", (0, 0), (-1, -1), 0.25, colors.HexColor("#CCCCCC")),
                ("TOPPADDING", (0, 0), (-1, -1), 5),
                ("BOTTOMPADDING", (0, 0), (-1, -1), 5),
            ]
            if header:
                style += [("BACKGROUND", (0, 0), (-1, 0), colors.HexColor("#F2F2F2")),
                          ("LINEBELOW", (0, 0), (-1, 0), 0.8, colors.HexColor("#808080"))]
            for j, al in enumerate(b.get("align") or []):
                style.append(("ALIGN", (j, 0), (j, -1), al.upper()))
            t.setStyle(TableStyle(style))
            out += [t, Spacer(1, 8)]
        elif kind == "image":
            path = b.get("path", "")
            if not os.path.isabs(path):
                path = os.path.join(base_dir, path)
            img = Image(path)
            w = min(float(b.get("width", 6.0)) * inch, frame_width)
            img.drawHeight, img.drawWidth = img.drawHeight * w / img.drawWidth, w
            out.append(img)
            if b.get("caption"):
                out.append(Paragraph(b["caption"], st["caption"]))
        elif kind == "spacer":
            out.append(Spacer(1, float(b.get("height", 0.25)) * inch))
        elif kind == "page_break":
            out.append(PageBreak())
        else:
            raise SystemExit(f"unknown block type: {kind}")
    return out


def page_size(spec):
    size = A4 if str(spec.get("pagesize", "letter")).lower() == "a4" else LETTER
    return landscape(size) if spec.get("landscape") else size


class NumberedCanvas(canvas.Canvas):
    """Draws "Page N of M" once the page count is known."""

    def __init__(self, *a, **kw):
        super().__init__(*a, **kw)
        self._pages = []

    def showPage(self):
        self._pages.append(dict(self.__dict__))
        self._startPage()

    def save(self):
        total = len(self._pages)
        for state in self._pages:
            self.__dict__.update(state)
            self.setFont("Helvetica", 8.5)
            self.setFillColor(colors.HexColor("#666666"))
            self.drawCentredString(self._pagesize[0] / 2, 0.5 * inch, f"Page {self._pageNumber} of {total}")
            super().showPage()
        super().save()


def read_input(path):
    """The input file's text; `-` reads standard input."""
    if path == "-":
        return sys.stdin.read()
    with open(path, encoding="utf-8") as f:
        return f.read()


def is_json(path, src):
    return path.lower().endswith(".json") or (path == "-" and src.lstrip().startswith("{"))


def cmd_create(a):
    src = read_input(a.input)
    spec = json.loads(src) if is_json(a.input, src) else {"blocks": markdown_blocks(src)}
    if a.title:
        spec["title"] = a.title
    size = page_size(spec)
    doc = SimpleDocTemplate(a.output, pagesize=size, leftMargin=inch, rightMargin=inch, topMargin=inch, bottomMargin=inch,
                            title=spec.get("title", ""), author=spec.get("author", ""))
    st = styles()
    story = []
    if spec.get("title"):
        story.append(Paragraph(md_inline(spec["title"]), st["title"]))
    story += flowables(spec.get("blocks", []), st, os.path.dirname(os.path.abspath(a.input)), doc.width)
    footer = spec.get("footer")
    if footer == "page":
        doc.build(story, canvasmaker=NumberedCanvas)
    elif footer:
        def draw(c, d):
            c.setFont("Helvetica", 8.5)
            c.setFillColor(colors.HexColor("#666666"))
            c.drawCentredString(size[0] / 2, 0.5 * inch, footer)
        doc.build(story, onFirstPage=draw, onLaterPages=draw)
    else:
        doc.build(story)
    print(f"wrote {a.output}")


def cmd_form(a):
    spec = json.loads(read_input(a.input))
    size = page_size(spec)
    c = canvas.Canvas(a.output, pagesize=size)
    c.setTitle(spec.get("title", "Form"))
    y = size[1] - inch
    if spec.get("title"):
        c.setFont("Helvetica-Bold", 20)
        c.drawString(inch, y, spec["title"])
        y -= 0.6 * inch
    form = c.acroForm
    for fld in spec.get("fields", []):
        if y < 1.2 * inch:
            c.showPage()
            y = size[1] - inch
        c.setFont("Helvetica", 10)
        c.setFillColor(colors.black)
        kind = fld.get("type", "text")
        if kind == "checkbox":
            form.checkbox(name=fld["name"], x=inch, y=y - 14, size=14, borderColor=colors.HexColor("#808080"),
                          fillColor=colors.white, buttonStyle="check", checked=bool(fld.get("value")))
            c.drawString(inch + 22, y - 11, fld.get("label", fld["name"]))
            y -= 0.5 * inch
        else:
            c.drawString(inch, y, fld.get("label", fld["name"]))
            form.textfield(name=fld["name"], x=inch, y=y - 26, width=size[0] - 2 * inch, height=20,
                           value=str(fld.get("value", "")), borderColor=colors.HexColor("#808080"),
                           fillColor=colors.white, textColor=colors.black, forceBorder=True, fontSize=10)
            y -= 0.75 * inch
    c.save()
    print(f"wrote {a.output} ({len(spec.get('fields', []))} field(s))")


def page_list(spec, count):
    """'1-3,5,7-' (1-based) → zero-based indices in order."""
    out = []
    for part in spec.split(","):
        part = part.strip()
        if not part:
            continue
        if "-" in part:
            lo, hi = part.split("-", 1)
            lo = int(lo) if lo else 1
            hi = int(hi) if hi else count
            out += list(range(lo - 1, min(hi, count)))
        else:
            out.append(int(part) - 1)
    bad = [i + 1 for i in out if not 0 <= i < count]
    if bad:
        raise SystemExit(f"page(s) {bad} out of range (1..{count})")
    return out


def cmd_info(a):
    r = PdfReader(a.file)
    box = r.pages[0].mediabox if r.pages else None
    print(f"pages: {len(r.pages)}")
    if box:
        print(f"page size: {float(box.width) / 72:.2f} x {float(box.height) / 72:.2f} in")
    meta = r.metadata or {}
    for k in ("/Title", "/Author", "/Creator", "/Producer"):
        if meta.get(k):
            print(f"{k[1:].lower()}: {meta[k]}")
    fields = r.get_fields() or {}
    print(f"form fields: {len(fields)}")
    print(f"encrypted: {r.is_encrypted}")


def cmd_extract(a):
    r = PdfReader(a.file)
    idx = page_list(a.pages, len(r.pages)) if a.pages else range(len(r.pages))
    text = "\n\n".join(f"--- page {i + 1}\n{r.pages[i].extract_text() or ''}" for i in idx)
    if a.output:
        with open(a.output, "w", encoding="utf-8") as f:
            f.write(text)
        print(f"wrote {a.output}")
    else:
        print(text)


def cmd_merge(a):
    w = PdfWriter()
    for path in a.files:
        w.append(path)
    with open(a.output, "wb") as f:
        w.write(f)
    print(f"wrote {a.output} ({len(w.pages)} pages)")


def cmd_split(a):
    r = PdfReader(a.file)
    os.makedirs(a.outdir, exist_ok=True)
    stem = os.path.splitext(os.path.basename(a.file))[0]
    for i, page in enumerate(r.pages, 1):
        w = PdfWriter()
        w.add_page(page)
        path = os.path.join(a.outdir, f"{stem}-{i:03d}.pdf")
        with open(path, "wb") as f:
            w.write(f)
    print(f"wrote {len(r.pages)} file(s) to {a.outdir}")


def cmd_pages(a):
    r = PdfReader(a.file)
    w = PdfWriter()
    for i in page_list(a.spec, len(r.pages)):
        w.add_page(r.pages[i])
    with open(a.output, "wb") as f:
        w.write(f)
    print(f"wrote {a.output} ({len(w.pages)} pages)")


def cmd_rotate(a):
    r = PdfReader(a.file)
    w = PdfWriter()
    chosen = set(page_list(a.pages, len(r.pages))) if a.pages else set(range(len(r.pages)))
    for i, page in enumerate(r.pages):
        if i in chosen:
            page.rotate(a.degrees)
        w.add_page(page)
    with open(a.output or a.file, "wb") as f:
        w.write(f)
    print("ok")


def cmd_fields(a):
    fields = PdfReader(a.file).get_fields() or {}
    if not fields:
        print("no form fields")
    for name, fld in fields.items():
        print(f"{name} ({fld.get('/FT', '?')}): {fld.get('/V', '')}")


def cmd_fill(a):
    r = PdfReader(a.file)
    w = PdfWriter(clone_from=r)
    fields = r.get_fields() or {}
    values = {}
    for kv in a.values:
        if "=" not in kv:
            raise SystemExit(f"expected name=value, got {kv!r}")
        k, v = kv.split("=", 1)
        if k not in fields:
            raise SystemExit(f"no field named {k!r} (have {', '.join(fields)})")
        if fields[k].get("/FT") == "/Btn":
            states = [s for s in (fields[k].get("/_States_", []) or []) if s != "/Off"]
            on = states[0] if states else "/Yes"
            v = on if v.lower() in ("1", "true", "yes", "on", "x") else "/Off"
        values[k] = v
    for page in w.pages:
        w.update_page_form_field_values(page, values, auto_regenerate=False)
    w.set_need_appearances_writer(True)
    if a.flatten:
        for page in w.pages:
            w.update_page_form_field_values(page, values, flatten=True)
        if "/AcroForm" in w._root_object:
            del w._root_object[NameObject("/AcroForm")]
    with open(a.output, "wb") as f:
        w.write(f)
    print(f"filled {len(values)} field(s) → {a.output}")


def cmd_watermark(a):
    r = PdfReader(a.file)
    w = PdfWriter()
    for page in r.pages:
        pw, ph = float(page.mediabox.width), float(page.mediabox.height)
        buf = BytesIO()
        c = canvas.Canvas(buf, pagesize=(pw, ph))
        c.setFont("Helvetica-Bold", min(pw, ph) / 7)
        c.setFillColor(colors.Color(0, 0, 0, alpha=0.08))
        c.translate(pw / 2, ph / 2)
        c.rotate(45)
        c.drawCentredString(0, 0, a.text)
        c.save()
        buf.seek(0)
        page.merge_page(PdfReader(buf).pages[0])
        w.add_page(page)
    with open(a.output or a.file, "wb") as f:
        w.write(f)
    print("ok")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("create", help="Markdown/JSON → PDF")
    c.add_argument("input")
    c.add_argument("output")
    c.add_argument("--title")
    c.set_defaults(fn=cmd_create)
    fm = sub.add_parser("form", help="JSON → fillable PDF form")
    fm.add_argument("input")
    fm.add_argument("output")
    fm.set_defaults(fn=cmd_form)
    i = sub.add_parser("info")
    i.add_argument("file")
    i.set_defaults(fn=cmd_info)
    e = sub.add_parser("extract", help="extract text")
    e.add_argument("file")
    e.add_argument("--pages")
    e.add_argument("-o", "--output")
    e.set_defaults(fn=cmd_extract)
    m = sub.add_parser("merge")
    m.add_argument("files", nargs="+")
    m.add_argument("-o", "--output", required=True)
    m.set_defaults(fn=cmd_merge)
    s = sub.add_parser("split")
    s.add_argument("file")
    s.add_argument("outdir")
    s.set_defaults(fn=cmd_split)
    p = sub.add_parser("pages", help="select/reorder pages")
    p.add_argument("file")
    p.add_argument("spec")
    p.add_argument("-o", "--output", required=True)
    p.set_defaults(fn=cmd_pages)
    r = sub.add_parser("rotate")
    r.add_argument("file")
    r.add_argument("degrees", type=int)
    r.add_argument("--pages")
    r.add_argument("-o", "--output")
    r.set_defaults(fn=cmd_rotate)
    f = sub.add_parser("fields", help="list form fields")
    f.add_argument("file")
    f.set_defaults(fn=cmd_fields)
    fl = sub.add_parser("fill", help="fill form fields: name=value ...")
    fl.add_argument("file")
    fl.add_argument("output")
    fl.add_argument("values", nargs="+")
    fl.add_argument("--flatten", action="store_true")
    fl.set_defaults(fn=cmd_fill)
    wm = sub.add_parser("watermark")
    wm.add_argument("file")
    wm.add_argument("text")
    wm.add_argument("-o", "--output")
    wm.set_defaults(fn=cmd_watermark)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
