"""Keplorayx's name in what agents read from the engine's MCP server: every
"Zeron" inside a Rust string literal in zr/crates/mcp/src becomes
"Keplorayx" (tool descriptions, the server's instructions, its answers).
Code (types, paths, crate names) is left alone: only text between quotes,
and the continued lines of a multi-line string, change. Run from the
repository's root."""

import pathlib
import re

ROOT = pathlib.Path('zr/crates/mcp/src')
BACKSLASH = chr(92)


def rename_strings(text):
    out = []
    i, n = 0, len(text)
    inside = False
    while i < n:
        c = text[i]
        if not inside:
            # A comment's to the line's end; a char literal like '"' isn't a string.
            if text.startswith('//', i):
                j = text.find('\n', i)
                j = n if j < 0 else j
                out.append(text[i:j])
                i = j
                continue
            if c == "'" and i + 2 < n and text[i + 2] == "'":
                out.append(text[i:i + 3])
                i += 3
                continue
            if c == '"' and not (i > 0 and text[i - 1] == 'r'):
                inside = True
                start = i + 1
                out.append(c)
                i += 1
                continue
            out.append(c)
            i += 1
            continue
        if c == BACKSLASH:
            i += 2
            continue
        if c == '"':
            literal = text[start:i]
            literal = literal.replace("Zeron's", "Keplorayx's").replace('Zeron', 'Keplorayx')
            out.append(literal)
            out.append(c)
            inside = False
            i += 1
            continue
        i += 1
    return ''.join(out)


for path in list(ROOT.glob('*.rs')) + list(ROOT.glob('tools/*.rs')):
    before = path.read_text(encoding='utf-8')
    after = rename_strings(before)
    if after != before:
        path.write_text(after, encoding='utf-8', newline='\n')
        print(path, before.count('Zeron') - after.count('Zeron'))
