"""Confere o quadro da amostra-grade.sh (item E1b).

Uso: `conferir-grade.py quadro.png [--sem-icones]` (sem Nerd Font instalada, os ícones
não são conferidos).

Acha a grade pelos blocos vermelhos da linha 0 e confere linhas de quadro contínuas,
blocos sem frestas, a seta do powerline encostando nas células vizinhas, o negrito na
mesma posição do normal e os ícones dentro das células. Sai com 0 se tudo certo; os
erros vão para a saída.
"""
import sys

from PIL import Image

im = Image.open(sys.argv[1]).convert("RGB")
W, H = im.size
px = im.load()
erros = []


def near(c, alvo, tol=40):
    return all(abs(a - b) <= tol for a, b in zip(c, alvo))


# Grade: os 10 blocos vermelhos da linha 0 (só na metade esquerda de cima da tela).
reds = [(x, y) for y in range(28, H // 2) for x in range(0, W // 2) if px[x, y] == (255, 0, 0)]
if not reds:
    print("sem os blocos vermelhos")
    sys.exit(1)
x0, y0 = min(p[0] for p in reds), min(p[1] for p in reds)
x1, y1 = max(p[0] for p in reds) + 1, max(p[1] for p in reds) + 1
cw, ch = (x1 - x0) / 10, y1 - y0
if cw != int(cw):
    erros.append("largura dos blocos %d não é 10 células" % (x1 - x0))
cw = int(cw)
print("grade: origem (%d, %d), célula %dx%d" % (x0, y0, cw, ch))
fundo = px[x0 + 20 * cw, y0 + 5 * ch + ch // 2]

# Blocos: todo o retângulo vermelho.
falhas = sum(1 for y in range(y0, y1) for x in range(x0, x1) if px[x, y] != (255, 0, 0))
if falhas:
    erros.append("blocos com %d pixels sem cor (frestas)" % falhas)


def linha(n):
    return y0 + n * ch


# Linha horizontal: alguma linha de pixels azul de ponta a ponta das 10 células.
cheias = [y for y in range(linha(1), linha(2)) if all(near(px[x, y], (0, 0, 255), 60) for x in range(x0, x0 + 10 * cw))]
if not cheias:
    erros.append("── com falhas entre as células")

# Linha vertical: alguma coluna verde de cima a baixo das linhas 2 a 4.
cheias = [x for x in range(x0, x0 + cw) if all(near(px[x, y], (0, 160, 0), 60) for y in range(linha(2), linha(5)))]
if not cheias:
    erros.append("│ ├ │ sem se encontrar entre as linhas")

# Powerline: nada da cor do fundo entre o verde e o azul, a coluna da esquerda da seta
# toda verde e a ponta chegando ao fim da célula.
band = range(linha(5), linha(6))
furos = [(x, y) for y in band for x in range(x0, x0 + 5 * cw) if near(px[x, y], fundo, 30) and not near(fundo, (0, 160, 0), 30)]
if furos:
    erros.append("powerline com %d pixels do fundo (frestas), ex. %s" % (len(furos), furos[:3]))
seta = x0 + 2 * cw
if not all(near(px[seta, y], (0, 160, 0), 50) for y in band):
    erros.append("seta do powerline não encosta na célula da esquerda")
meio = linha(5) + ch // 2
if not near(px[seta + cw - 1, meio], (0, 160, 0), 110):
    erros.append("ponta da seta não chega à borda direita: %s" % (px[seta + cw - 1, meio],))


# "i" normal e em negrito: o centro da tinta de cada célula no mesmo lugar (±1).
def centros(n):
    out = []
    for c in range(10):
        xs = [x for x in range(x0 + c * cw, x0 + (c + 1) * cw) for y in range(linha(n), linha(n + 1)) if sum(px[x, y]) < 300]
        out.append(sum(xs) / len(xs) - (x0 + c * cw) if xs else None)
    return out


normal, negrito = centros(6), centros(7)
if None in normal or None in negrito:
    erros.append("letras sumidas: %s %s" % (normal, negrito))
elif any(abs(a - b) > 1.0 for a, b in zip(normal, negrito)):
    erros.append("negrito fora da grade: normal %s, negrito %s" % ([round(v, 1) for v in normal], [round(v, 1) for v in negrito]))
elif max(normal) - min(normal) > 0.01:
    erros.append("letras em posições diferentes nas células: %s" % normal)

# Ícones (magenta), se houver Nerd Font: o primeiro dentro das células 0 e 1, o segundo só na célula 2; os
# dois dentro da altura da linha.
sem_icones = "--sem-icones" in sys.argv
mag = [] if sem_icones else [(x, y) for y in range(linha(8) - ch, linha(10)) for x in range(x0 - cw, x0 + 6 * cw) if near(px[x, y], (200, 0, 200), 70)]
a = [p for p in mag if p[0] < x0 + 2 * cw]
b = [p for p in mag if p[0] >= x0 + 2 * cw]
if sem_icones:
    pass
elif len(a) < 10 or len(b) < 10:
    erros.append("ícones sumidos (%d e %d pixels)" % (len(a), len(b)))
else:
    if min(p[0] for p in a) < x0 or max(p[0] for p in a) >= x0 + 2 * cw:
        erros.append("primeiro ícone fora das duas células")
    if min(p[0] for p in b) < x0 + 2 * cw or max(p[0] for p in b) >= x0 + 3 * cw:
        erros.append("segundo ícone fora da célula (invade o x)")
    if min(p[1] for p in mag) < linha(8) or max(p[1] for p in mag) >= linha(9):
        erros.append("ícones fora da altura da linha")
    print("ícones: %d e %d pixels de largura" % (max(p[0] for p in a) - min(p[0] for p in a) + 1, max(p[0] for p in b) - min(p[0] for p in b) + 1))

for e in erros:
    print("erro:", e)
sys.exit(1 if erros else 0)
