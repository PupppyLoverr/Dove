# Office skills

Agent Skills bundles (one folder per skill, `SKILL.md` + `scripts/`) for
professional office documents. Claude Code, Codex, OpenCode and every other
harness that reads the [Agent Skills](https://agentskills.io) format can load
them as is.

| Skill | What it does | Library |
| --- | --- | --- |
| [`pdf`](pdf/SKILL.md) | create, merge, split, rotate, extract text, list and fill form fields | reportlab, pypdf |
| [`pptx`](pptx/SKILL.md) | decks from outlines on real slide layouts, dump and edit slides | python-pptx |
| [`xlsx`](xlsx/SKILL.md) | workbooks with formulas, number formats, styling and charts; edit cells | openpyxl |
| [`docx`](docx/SKILL.md) | Word documents with styles, lists, tables and images; edit text | python-docx |

All libraries are free and open source.

## Bootstrap

Every script carries [PEP 723](https://peps.python.org/pep-0723/) inline
metadata, so with [uv](https://docs.astral.sh/uv/) installed it runs with no
setup; uv fetches the dependencies into a cached environment on first use:

```sh
uv run skills/pptx/scripts/pptx_tool.py create outline.md deck.pptx
```

Without uv, install the dependencies once (Python 3.9+) and run the script
with `python3`:

```sh
python3 -m pip install --user python-docx python-pptx openpyxl reportlab pypdf
```

## How Keplorayx ships them

The Keplorayx binary embeds this folder. Before it starts any agent, Keplorayx writes the
skills to `~/.dove/skills/<name>` (or `$DOVE_HOME/skills`) and links them where
the harnesses look for user skills:

- `~/.claude/skills/<name>` (or `$CLAUDE_CONFIG_DIR/skills`) for Claude Code and OpenCode
- `~/.agents/skills/<name>` for Codex, OpenCode, Cursor, Devin, Grok, Pi and other Agent Skills readers

A folder that already exists there and was not written by Keplorayx is left alone.
ACP agents that do not read skill folders get them through the `dove` MCP
server: its instructions name the skills and the `dove_skills` tool returns a
skill's full text and script paths. Turn the install off with the
`agent_office_skills` setting set to `false`.

Keplorayx's file viewer previews and edits `.pdf`, `.docx`, `.xlsx` and `.pptx`
natively, so the user can check what the agent made in the side panel.
