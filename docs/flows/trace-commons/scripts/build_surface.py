#!/usr/bin/env python3
"""Render the flow review surface from model frames and the app's design tokens."""
import json
from pathlib import Path

FLOW = Path(__file__).resolve().parent.parent
REPO = FLOW.parents[2]


def build():
    models = []
    for file in sorted((FLOW / 'models').glob('*.frames.json')):
        name = file.name.removesuffix('.frames.json')
        models.append({'name': name, 'frames': json.loads(file.read_text()),
                       'chart': f'../models/{name}.svg' if (FLOW / 'models' / f'{name}.svg').is_file() else None})
    tokens = json.loads((REPO / 'design-tokens/glass.tokens.json').read_text())
    colors = dict(tokens['color'])
    colors.update(tokens['themes']['flatDark']['color'])
    css = []
    for name, value in colors.items():
        if 'hex' in value:
            raw = value['hex'].lstrip('#')
            r, g, b = (int(raw[i:i+2], 16) for i in (0, 2, 4))
            css.append(f'--{name}:rgba({r},{g},{b},{value.get("alpha", 1)});')
    data = json.dumps(models, ensure_ascii=False).replace('<', '\\u003c')
    template = (FLOW / 'scripts/surface.html').read_text()
    rendered = template.replace('__TOKENS__', ''.join(css)).replace('__DATA__', data)
    (FLOW / 'mocks/index.html').write_text(rendered)
    print(f'Rendered {len(models)} model families / {sum(len(m["frames"]) for m in models)} frames')


if __name__ == '__main__':
    build()
