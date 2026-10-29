---
name: docx
description: Create and edit Word documents (.docx) with real styles, headings, bullet and numbered lists, tables, images, headers and footers; dump a document's text and structure; replace or rewrite paragraphs without losing formatting. Use whenever the user asks for a Word document, report, letter, memo, proposal or any .docx file, or wants an existing .docx read or changed.
---

# Word documents (.docx)

Script: `scripts/docx_tool.py` (python-docx). Run it with `uv run scripts/docx_tool.py ...`

Input files (specs, outlines) can be `-` to read from standard input, e.g. `... create - out.docx <<'EOF'`.
(uv installs the dependency on first use) or `python3 scripts/docx_tool.py ...` after
`python3 -m pip install --user python-docx`. Paths below are relative to this skill folder.

## Create

From Markdown (fastest for prose):

```sh
uv run scripts/docx_tool.py create report.md report.docx --title "Q3 Report"
```

Supported Markdown: `#`..`####` headings, paragraphs, `**bold**`, `*italic*`, `` `code` ``,
`-`/`*` bullets (indent two spaces to nest), `1.` numbered items, `> quotes`, pipe tables
(first row is the header), `![caption](image.png)`, and a line containing only
`\pagebreak` for a page break.

From JSON (full control) — same command with a `.json` input:

```json
{
  "title": "Proposal", "subtitle": "Prepared for Acme", "author": "Dove",
  "font": "Calibri", "size": 11, "header": "Acme · Confidential", "footer": "page",
  "blocks": [
    {"type": "heading", "text": "Summary", "level": 1},
    {"type": "paragraph", "text": "Plain text, or use runs:", "align": "justify"},
    {"type": "paragraph", "runs": [{"text": "Bold ", "bold": true}, {"text": "italic", "italic": true}]},
    {"type": "bullets", "items": ["One", "Two", ["Nested a", "Nested b"]]},
    {"type": "numbered", "items": ["First", "Second"]},
    {"type": "table", "header": ["Item", "Cost"], "rows": [["Design", "$4,000"]], "widths": [3.5, 1.5]},
    {"type": "image", "path": "chart.png", "width": 5.5, "caption": "Figure 1"},
    {"type": "quote", "text": "A pull quote."},
    {"type": "page_break"}
  ]
}
```

`"footer": "page"` puts a centered page number in the footer; any other string is literal text.
`--template base.docx` starts from an existing document's styles (its body is cleared).

## Read

```sh
uv run scripts/docx_tool.py dump report.docx          # numbered paragraphs with style names, then tables
uv run scripts/docx_tool.py dump report.docx --json   # machine-readable
```

## Edit (keeps styles and run formatting)

```sh
uv run scripts/docx_tool.py replace report.docx "2023" "2024" -o out.docx   # find/replace in body, tables, headers
uv run scripts/docx_tool.py set report.docx 12 "New text for paragraph 12"   # index from `dump`
uv run scripts/docx_tool.py append report.docx more.md                     # add Markdown/JSON blocks at the end
uv run scripts/docx_tool.py delete report.docx 7
```

Without `-o` the input file is rewritten in place.

## Rules for good documents

- Use real heading styles (never bold body text as a heading) so navigation and TOCs work.
- One idea per paragraph; tables for comparisons; keep tables under ~6 columns.
- Put images next to the text that refers to them, at most the text width (6.5in on Letter).
- After writing, run `dump` to check the result before telling the user it is done.
- Keplorayx's side panel previews and edits .docx files: tell the user the path so they can open it.
