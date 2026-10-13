---
name: pptx
description: Create PowerPoint decks (.pptx) from an outline on the deck's real slide layouts (title, bullets, two columns, section, image, table, quote, closing), with speaker notes and 16:9 sizing; dump a deck's text; edit slide titles and text, replace text, add or delete slides. Use whenever the user asks for slides, a presentation, a pitch deck or any .pptx file, or wants an existing deck read or changed.
---

# PowerPoint decks (.pptx)

Script: `scripts/pptx_tool.py` (python-pptx). Run it with `uv run scripts/pptx_tool.py ...`
(uv installs the dependency on first use) or `python3 scripts/pptx_tool.py ...` after
`python3 -m pip install --user python-pptx`. Paths below are relative to this skill folder.

## Create from an outline

```sh
uv run scripts/pptx_tool.py create outline.md deck.pptx
uv run scripts/pptx_tool.py create outline.md deck.pptx --template brand.pptx   # use a company template's layouts
```

Markdown outline:

```markdown
# Deck title
Subtitle line

## Agenda
- Problem
- Solution
  - Nested point
> Speaker notes go in quote lines.

## Before | After          <- a "|" in the title makes a two-column slide;
- Manual                    <- bullets before a lone "---" go left, after go right
---
- Automated

## Revenue
![Revenue chart](chart.png)

## Pricing
| Plan | Price |
| ---- | ----- |
| Free | $0    |

# Section title             <- later "#" headings become section slides
```

JSON (full control) — same command with a `.json` input:

```json
{"title": "Dove", "subtitle": "Native agent workspace", "widescreen": true,
 "slides": [
   {"layout": "bullets", "title": "Why", "bullets": ["Fast", ["Native", "GPU"], "Local"], "notes": "..."},
   {"layout": "two_column", "title": "Compare", "left": ["A"], "right": ["B"]},
   {"layout": "section", "title": "Part two"},
   {"layout": "image", "title": "Screenshot", "image": "shot.png", "caption": "Main window"},
   {"layout": "table", "title": "Plans", "header": ["Plan", "Price"], "rows": [["Free", "$0"]]},
   {"layout": "quote", "text": "It just works.", "by": "A user"},
   {"layout": "closing", "title": "Thank you", "subtitle": "hello@example.com"}
 ]}
```

Slides use the template's own layouts and placeholders (Title Slide, Title and Content,
Two Content, Section Header, Title Only), so the deck stays editable in PowerPoint, Keynote
and Google Slides. The default template is resized to 16:9 with its layouts rescaled.

## Read and edit

```sh
uv run scripts/pptx_tool.py dump deck.pptx            # slide N: layout, every shape's text, notes
uv run scripts/pptx_tool.py dump deck.pptx --json
uv run scripts/pptx_tool.py set-title deck.pptx 3 "New title"
uv run scripts/pptx_tool.py set-text deck.pptx 3 1 "Line one\nLine two"   # slide 3, shape 1 (from dump)
uv run scripts/pptx_tool.py replace deck.pptx "2023" "2024"
uv run scripts/pptx_tool.py add deck.pptx more.md    # append slides from an outline
uv run scripts/pptx_tool.py delete deck.pptx 4
uv run scripts/pptx_tool.py notes deck.pptx 2 "Speaker notes"
```

Slides are numbered from 1. Edits keep each paragraph's formatting. Add `-o out.pptx` to write
a copy instead of rewriting the file.

## Rules for good decks

- One message per slide; the title states it. At most ~6 bullets, ~8 words each.
- Use section slides to group a long deck; finish with a closing slide.
- Prefer an image or table slide to a wall of bullets. Put detail in speaker notes.
- After writing, run `dump` to check the result. Dove's side panel previews and edits .pptx
  files: tell the user the path.
