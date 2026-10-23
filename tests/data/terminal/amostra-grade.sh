#!/bin/sh
# Amostra do item E1b (tests/ui.sh, seção do terminal): cada linha é conferida pixel
# a pixel por conferir-grade.py. Cores RGB exatas para achar cada parte no quadro.
R='\033[38;2;255;0;0m'; B='\033[38;2;0;0;255m'; G='\033[38;2;0;160;0m'
P='\033[38;2;0;0;0m'; M='\033[38;2;200;0;200m'; Z='\033[0m'
printf '\033[?25l\033[2J\033[H'
# 0: blocos cheios (sem frestas entre células)
printf "${R}██████████${Z}\n"
# 1: linha horizontal (contínua entre células)
printf "${B}──────────${Z}\n"
# 2-4: linha vertical com um ├ no meio (contínua entre linhas)
printf "${G}│${Z}\n${G}├─${Z}\n${G}│${Z}\n"
# 5: powerline: verde, seta verde sobre azul, azul
printf '\033[48;2;0;160;0m  \033[38;2;0;160;0;48;2;0;0;255m\356\202\260\033[48;2;0;0;255m  \033[0m\n'
# 6-7: "i" normal e em negrito (na mesma posição da célula)
printf "${P}iiiiiiiiii${Z}\n"
printf "${P}\033[1miiiiiiiiii${Z}\n"
# 8: ícone com espaço depois (até duas células) e ícone com letra depois (uma célula)
printf "${M}\356\227\277 \356\227\277${P}x${Z}\n"
sleep 60
