#!/usr/bin/env python3
"""Gera os ícones de arquivo do Code (mide-code) a partir de um clone do Material
Icon Theme (MIT, github.com/material-extensions/vscode-material-icon-theme).

Uso: python3 data/code-icons/gen.py ~/Projetos/vscode-material-icon-theme

Copia os SVGs usados para data/code-icons/svg/ e escreve src/code/file_icons_gen.rs
com as tabelas (extensão, nome de arquivo e nome de pasta → ícone) e os SVGs
embutidos (include_bytes!).
"""
import os, re, shutil, sys

src = sys.argv[1]
here = os.path.dirname(os.path.abspath(__file__))
root = os.path.dirname(os.path.dirname(here))
icons_dir = os.path.join(src, 'icons')


def entries(text):
    """Cada objeto { name: '...', ... } de nível de ícone, com as listas dele."""
    out = []
    for m in re.finditer(r"\{\s*name:\s*'([^']+)'", text):
        start = m.start()
        depth, i = 0, start
        while i < len(text):
            c = text[i]
            if c == '{':
                depth += 1
            elif c == '}':
                depth -= 1
                if depth == 0:
                    break
            i += 1
        body = text[start:i]
        # Objetos com outros objetos dentro (temas de pasta) não são ícones.
        if body.count('{') > 1 and 'icons:' in body:
            continue
        def arr(key):
            a = re.search(key + r":\s*\[([^\]]*)\]", body)
            return re.findall(r"'([^']+)'", a.group(1)) if a else []
        out.append({
            'name': m.group(1),
            'ext': arr('fileExtensions'),
            'files': arr('fileNames'),
            'folders': arr('folderNames'),
            'light': bool(re.search(r"\blight:\s*true", body)),
            'disabled': 'disabled: true' in body,
        })
    return out


files = entries(open(os.path.join(src, 'src/core/icons/fileIcons.ts')).read())
folders_text = open(os.path.join(src, 'src/core/icons/folderIcons.ts')).read()
# Só o tema de pastas "specific" (o padrão do tema).
spec = folders_text.split("name: 'classic'")[0]
folders = entries(spec)
langs = entries(open(os.path.join(src, 'src/core/icons/languageIcons.ts')).read())

ext_map, name_map, folder_map, used = {}, {}, {}, set()
for e in files:
    if e['disabled'] or not os.path.exists(os.path.join(icons_dir, e['name'] + '.svg')):
        continue
    for x in e['ext']:
        ext_map.setdefault(x.lower(), e['name'])
    for f in e['files']:
        name_map.setdefault(f.lower(), e['name'])
    if e['ext'] or e['files']:
        used.add(e['name'])
for e in folders:
    if not e['folders'] or not os.path.exists(os.path.join(icons_dir, e['name'] + '.svg')):
        continue
    for f in e['folders']:
        folder_map.setdefault(f.lower(), e['name'])
    used.add(e['name'])
    if os.path.exists(os.path.join(icons_dir, e['name'] + '-open.svg')):
        used.add(e['name'] + '-open')
# Extensões das linguagens (ids do VS Code iguais às extensões mais comuns).
for e in langs:
    if os.path.exists(os.path.join(icons_dir, e['name'] + '.svg')):
        pass
used |= {'file', 'folder', 'folder-open', 'folder-root', 'folder-root-open'}
# Variantes claras (para o tema claro), quando existem.
for n in list(used):
    if n not in ('file', 'folder', 'folder-open', 'folder-root', 'folder-root-open') and os.path.exists(os.path.join(icons_dir, n + '_light.svg')):
        used.add(n + '_light')

out_svg = os.path.join(here, 'svg')
shutil.rmtree(out_svg, ignore_errors=True)
os.makedirs(out_svg)
# Os padrões (arquivo, pasta, pasta raiz) o tema gera no build dele, na cor padrão
# #90a4ae (src/core/generator): os mesmos desenhos, escritos aqui.
GREY = '#90a4ae'
def svg(body):
    return f'<svg viewBox="0 0 16 16" xmlns="http://www.w3.org/2000/svg">{body}</svg>'
def path(d):
    return f'<path d="{d}" fill="{GREY}" />'
generated = {
    'file': svg(path('m8.668 6h3.6641l-3.6641-3.668v3.668m-4.668-4.668h5.332l4 4v8c0 0.73828-0.59375 1.3359-1.332 1.3359h-8c-0.73828 0-1.332-0.59766-1.332-1.3359v-10.664c0-0.74219 0.59375-1.3359 1.332-1.3359m3.332 1.3359h-3.332v10.664h8v-6h-4.668z')),
    'folder': svg(path('m6.922 3.768-.644-.536A1 1 0 0 0 5.638 3H2a1 1 0 0 0-1 1v8a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V5a1 1 0 0 0-1-1H7.562a1 1 0 0 1-.64-.232')),
    'folder-open': svg(path('M14.483 6H4.721a1 1 0 0 0-.949.684L2 12V5h12a1 1 0 0 0-1-1H7.562a1 1 0 0 1-.64-.232l-.644-.536A1 1 0 0 0 5.638 3H2a1 1 0 0 0-1 1v8a1 1 0 0 0 1 1h11l2.403-5.606A1 1 0 0 0 14.483 6')),
    'folder-root': svg(f'<circle cx="8" cy="8" r="6" fill="none" stroke="{GREY}" stroke-width="2"/><circle cx="8" cy="8" r="3" fill="{GREY}"/>'),
    'folder-root-open': svg(f'<circle cx="8" cy="8" r="6" fill="none" stroke="{GREY}" stroke-width="2"/>'),
}
for n in sorted(used):
    dst = os.path.join(out_svg, n + '.svg')
    if n in generated:
        open(dst, 'w').write(generated[n])
    else:
        shutil.copy(os.path.join(icons_dir, n + '.svg'), dst)
shutil.copy(os.path.join(src, 'LICENSE'), os.path.join(here, 'LICENSE'))


def table(name, m):
    rows = ''.join(f'    ({k!r}, {v!r}),\n'.replace("'", '"') for k, v in sorted(m.items()) if '"' not in k and '\\' not in k)
    return f'pub static {name}: &[(&str, &str)] = &[\n{rows}];\n'


rs = ['// Gerado por data/code-icons/gen.py a partir do Material Icon Theme (MIT).',
      '// Não editar à mão: rode o script de novo.', '']
rs.append(table('EXTENSIONS', ext_map))
rs.append(table('FILE_NAMES', name_map))
rs.append(table('FOLDER_NAMES', folder_map))
rs.append('pub static SVGS: &[(&str, &[u8])] = &[')
for n in sorted(used):
    rs.append(f'    ("{n}", include_bytes!("../../data/code-icons/svg/{n}.svg")),')
rs.append('];')
open(os.path.join(root, 'src/code/file_icons_gen.rs'), 'w').write('\n'.join(rs) + '\n')
print(f'{len(used)} ícones, {len(ext_map)} extensões, {len(name_map)} nomes, {len(folder_map)} pastas')
