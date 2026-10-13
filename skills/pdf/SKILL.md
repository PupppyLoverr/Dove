---
name: pdf
description: Create PDFs (reports, letters, invoices, fillable forms) from Markdown or JSON with real typography, tables and images; merge, split, rotate and reorder pages; extract text; list and fill AcroForm fields; stamp a watermark. Use whenever the user asks for a PDF, wants PDFs combined or split, needs text pulled out of a PDF, or wants a PDF form filled.
---

# PDF

Script: `scripts/pdf_tool.py` (reportlab to write, pypdf to read and edit). Run it with
`uv run scripts/pdf_tool.py ...` (uv installs the dependencies on first use) or
`python3 scripts/pdf_tool.py ...` after `python3 -m pip install --user reportlab pypdf`.
Paths below are relative to this skill folder.
Input files (specs, outlines) can be `-` to read from standard input, e.g. `... create - out.pdf <<'EOF'`.

## Create

```sh
uv run scripts/pdf_tool.py create report.md report.pdf --title "Quarterly report"
uv run scripts/pdf_tool.py create spec.json invoice.pdf
```

Markdown: `#`..`###` headings, paragraphs, `**bold**`, `*italic*`, `` `code` ``, `-` bullets,
`1.` numbered items, `> quotes`, pipe tables, `![caption](image.png)`, and `\pagebreak`.

JSON spec:

```json
{"title": "Invoice #42", "author": "Acme", "pagesize": "letter", "footer": "page",
 "blocks": [
   {"type": "heading", "text": "Bill to", "level": 2},
   {"type": "paragraph", "text": "Jane Doe<br/>1 Main St"},
   {"type": "table", "header": ["Item", "Qty", "Price"], "rows": [["Design", "1", "$4,000"]], "widths": [3.5, 1, 1.5], "align": ["LEFT", "RIGHT", "RIGHT"]},
   {"type": "bullets", "items": ["Net 30", "Thank you"]},
   {"type": "image", "path": "logo.png", "width": 2},
   {"type": "spacer", "height": 0.3},
   {"type": "page_break"}
 ]}
```

`pagesize`: `letter` (default) or `a4`; add `"landscape": true` to rotate. `footer: "page"` prints
"Page N of M". Paragraph text accepts reportlab mini-markup (`<b>`, `<i>`, `<br/>`, `<a href>`).

Fillable form:

```sh
uv run scripts/pdf_tool.py form form.json form.pdf
```

```json
{"title": "Registration", "fields": [
  {"name": "full_name", "label": "Full name"},
  {"name": "email", "label": "Email"},
  {"name": "agree", "label": "I agree", "type": "checkbox"}
]}
```

## Read and edit

```sh
uv run scripts/pdf_tool.py info file.pdf                     # pages, size, metadata, form fields
uv run scripts/pdf_tool.py extract file.pdf [--pages 1-3] [-o out.txt]
uv run scripts/pdf_tool.py merge a.pdf b.pdf c.pdf -o all.pdf
uv run scripts/pdf_tool.py split file.pdf outdir/            # one file per page
uv run scripts/pdf_tool.py pages file.pdf "3,1-2,5-" -o reordered.pdf   # select/reorder (1-based)
uv run scripts/pdf_tool.py rotate file.pdf 90 --pages 2 -o rotated.pdf
uv run scripts/pdf_tool.py fields form.pdf                   # list fillable fields and values
uv run scripts/pdf_tool.py fill form.pdf filled.pdf full_name="Jane Doe" agree=true
uv run scripts/pdf_tool.py watermark file.pdf "DRAFT" -o draft.pdf
```

## Notes

- Scanned PDFs have no text layer; `extract` returns little. Say so instead of guessing.
- `fill` sets field values and asks viewers to regenerate appearances, so filled values show in
  Preview, Acrobat and browsers. Add `--flatten` to make the result non-editable.
- Dove's side panel renders PDF pages: tell the user the path so they can check the output.
