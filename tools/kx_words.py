"""Keplorayx's name in the Zeron-based app's words: every user-facing string in
ui/locales says Keplorayx where Zeron's said Zeron. Keys and web addresses
(lowercase "zeron.sh") are left alone. Run from the repository root."""

import json

LOCALES = 'zr/apps/zgx/ui/locales'


def rename(v):
    if isinstance(v, dict):
        return {k: rename(x) for k, x in v.items()}
    if isinstance(v, list):
        return [rename(x) for x in v]
    if isinstance(v, str):
        return v.replace('Zeroning', 'Orbiting').replace('Zeron', 'Keplorayx')
    return v


for name in ['en.json', 'zh-Hans.json']:
    path = f'{LOCALES}/{name}'
    data = json.load(open(path, encoding='utf-8'))
    text = json.dumps(rename(data), indent=2, ensure_ascii=False) + '\n'
    open(path, 'w', encoding='utf-8', newline='\n').write(text)
    print(name, text.count('Keplorayx'))
