---
name: xlsx
description: Create and edit Excel workbooks (.xlsx) with live formulas, number formats, styled headers, column widths, frozen panes, filters, conditional fills and native charts (bar, column, line, pie, scatter); import CSV; dump a workbook's values and formulas; set cells or ranges. Use whenever the user asks for a spreadsheet, budget, model, tracker, table export or any .xlsx file, or wants an existing workbook read or changed.
---

# Excel workbooks (.xlsx)

Script: `scripts/xlsx_tool.py` (openpyxl). Run it with `uv run scripts/xlsx_tool.py ...`
(uv installs the dependency on first use) or `python3 scripts/xlsx_tool.py ...` after
`python3 -m pip install --user openpyxl`. Paths below are relative to this skill folder.

## Create

```sh
uv run scripts/xlsx_tool.py create spec.json budget.xlsx
uv run scripts/xlsx_tool.py from-csv data.csv data.xlsx --sheet Data
```

Spec:

```json
{"sheets": [{
  "name": "Budget",
  "columns": [
    {"header": "Item", "width": 24},
    {"header": "Q1", "width": 12, "format": "$#,##0"},
    {"header": "Q2", "width": 12, "format": "$#,##0"},
    {"header": "Total", "width": 14, "format": "$#,##0"}
  ],
  "rows": [
    ["Design", 4000, 5000, "=SUM(B2:C2)"],
    ["Hosting", 1200, 1300, "=SUM(B3:C3)"]
  ],
  "totals": {"label": "Total", "sum": ["B", "C", "D"]},
  "freeze": "A2", "filter": true,
  "charts": [{"type": "column", "title": "Spend by quarter", "data": "B1:C3", "categories": "A2:A3", "anchor": "F2"}],
  "highlight": [{"range": "D2:D3", "above": 5000, "fill": "EDEDED"}]
}]}
```

- A string starting with `=` is a formula; it stays live in Excel, Numbers, LibreOffice and Dove.
- `format` is any Excel number format (`0.0%`, `#,##0.00`, `yyyy-mm-dd`). ISO dates
  (`"2024-05-01"`) become real dates.
- `totals` appends a bold row with `SUM` formulas for the listed columns.
- Chart `type`: `bar`, `column`, `line`, `pie`, `scatter`. `data` includes the header row so
  series get names.
- Headers are bold on a light grey fill with a thin bottom border; keep it monochrome unless the
  user asks for colour.

## Read and edit

```sh
uv run scripts/xlsx_tool.py dump budget.xlsx                 # every sheet, values and formulas
uv run scripts/xlsx_tool.py dump budget.xlsx --sheet Budget --json
uv run scripts/xlsx_tool.py set budget.xlsx Budget!B2=4500 Budget!E1=Notes "Budget!D4==SUM(D2:D3)"
uv run scripts/xlsx_tool.py append budget.xlsx Budget '["Travel", 800, 900, "=SUM(B4:C4)"]'
uv run scripts/xlsx_tool.py add-sheet budget.xlsx spec.json   # add the spec's sheets to a workbook
```

`set` takes `Sheet!A1=value` (the sheet defaults to the first one). Numbers stay numbers; a value
starting with `=` is a formula. Styles on edited cells are kept.

## Notes

- openpyxl writes formulas without cached results. The workbook is flagged to recalculate on
  open, so Excel/LibreOffice show the values; Dove's viewer evaluates common functions (SUM,
  AVERAGE, MIN, MAX, COUNT, ROUND, IF, arithmetic) itself. Use `dump` to check formulas.
- Charts and formatting survive `set` and `append` because openpyxl rewrites the whole file;
  pivot tables and macros do not survive openpyxl at all — warn the user before editing such files.
- Dove's side panel previews and edits .xlsx files: tell the user the path.
