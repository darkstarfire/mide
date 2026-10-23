//! Telas (monitores) vistas por um app do mide-ui que cobre todas elas
//! (`Window::on_every_output`): geometria pura, sem Wayland, testável sozinha.
//!
//! Coordenadas "globais" são as do desktop inteiro, em pontos (a tela principal
//! costuma começar em (0, 0); a segunda, ao lado, em (1366, 0)). Coordenadas
//! "locais" são as de uma tela, a partir do canto dela.

/// Uma tela, em pontos (tamanho lógico, já dividido pela escala).
#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    /// Nome da saída (`HEADLESS-2`, `HDMI-A-1`).
    pub name: String,
    /// Canto superior esquerdo no desktop (global).
    pub origin: (f32, f32),
    /// Largura e altura em pontos.
    pub size: (f32, f32),
    /// Pixels por ponto (1, 1,25, 2...).
    pub scale: f32,
    /// É a tela principal (a da barra e da dock).
    pub primary: bool,
}

impl Screen {
    /// Retângulo global (x, y, largura, altura).
    pub fn rect(&self) -> (f32, f32, f32, f32) {
        (self.origin.0, self.origin.1, self.size.0, self.size.1)
    }

    /// O ponto global está nesta tela? (borda de cima e da esquerda incluídas.)
    pub fn contains(&self, p: (f32, f32)) -> bool {
        p.0 >= self.origin.0 && p.1 >= self.origin.1 && p.0 < self.origin.0 + self.size.0 && p.1 < self.origin.1 + self.size.1
    }

    /// Ponto global nas coordenadas desta tela.
    pub fn to_local(&self, p: (f32, f32)) -> (f32, f32) {
        (p.0 - self.origin.0, p.1 - self.origin.1)
    }

    /// Ponto desta tela nas coordenadas globais.
    pub fn to_global(&self, p: (f32, f32)) -> (f32, f32) {
        (p.0 + self.origin.0, p.1 + self.origin.1)
    }
}

/// Índice da tela que contém o ponto global (`None` num buraco entre telas).
pub fn screen_at(screens: &[Screen], p: (f32, f32)) -> Option<usize> {
    screens.iter().position(|s| s.contains(p))
}

/// Retângulo que cobre todas as telas (x, y, largura, altura); sem telas, zerado.
pub fn union(screens: &[Screen]) -> (f32, f32, f32, f32) {
    let Some(first) = screens.first() else {
        return (0.0, 0.0, 0.0, 0.0);
    };
    let (mut x0, mut y0) = first.origin;
    let (mut x1, mut y1) = (x0 + first.size.0, y0 + first.size.1);
    for s in &screens[1..] {
        x0 = x0.min(s.origin.0);
        y0 = y0.min(s.origin.1);
        x1 = x1.max(s.origin.0 + s.size.0);
        y1 = y1.max(s.origin.1 + s.size.1);
    }
    (x0, y0, x1 - x0, y1 - y0)
}

/// Telas espelhadas (mesmo retângulo no desktop) viram uma só: fica a principal, ou a
/// primeira da lista se nenhuma delas for a principal. A ordem das outras não muda.
/// Sem isso, um app de tela inteira poria duas camadas sobre a mesma área. Diferença
/// de até 2 pontos conta como igual: a cópia de 1920x1080 ajustada à largura de uma
/// principal de 1366x768 fica com 769 pontos de altura (arredondamento da escala).
pub fn dedupe_mirror(screens: Vec<Screen>) -> Vec<Screen> {
    dedupe_keeping(screens, |s| s.primary)
}

/// O mesmo que [`dedupe_mirror`], mas entre as espelhadas fica a que `keep` escolhe.
fn dedupe_keeping(screens: Vec<Screen>, keep: impl Fn(&Screen) -> bool) -> Vec<Screen> {
    let same = |a: (f32, f32, f32, f32), b: (f32, f32, f32, f32)| {
        (a.0 - b.0).abs() <= 2.0 && (a.1 - b.1).abs() <= 2.0 && (a.2 - b.2).abs() <= 2.0 && (a.3 - b.3).abs() <= 2.0
    };
    let mut out: Vec<Screen> = Vec::with_capacity(screens.len());
    for s in screens {
        match out.iter_mut().find(|o| same(o.rect(), s.rect())) {
            Some(existing) => {
                if keep(&s) && !keep(existing) {
                    *existing = s;
                }
            }
            None => out.push(s),
        }
    }
    out
}

/// Telas do app com a da superfície principal (`main`, o nome da saída onde ela está)
/// primeiro, sem as espelhadas (entre elas, fica a da superfície principal). A tela da
/// superfície principal não é sempre a principal do desktop: o compositor só escolhe a
/// saída de uma camada quando ela é criada, então trocar a principal com o app aberto
/// não muda a camada de lugar. `None` se a tela `main` não está na lista.
pub fn main_first(screens: Vec<Screen>, main: &str) -> Option<Vec<Screen>> {
    let mut screens = dedupe_keeping(screens, |s| s.name == main);
    let i = screens.iter().position(|s| s.name == main)?;
    let first = screens.remove(i);
    screens.insert(0, first);
    Some(screens)
}

/// A principal primeiro (é a superfície do app; as outras vêm depois, na ordem em que
/// chegaram). Se nenhuma estiver marcada, a primeira vira a principal.
pub fn primary_first(mut screens: Vec<Screen>) -> Vec<Screen> {
    if !screens.iter().any(|s| s.primary)
        && let Some(first) = screens.first_mut()
    {
        first.primary = true;
    }
    // `sort_by_key` é estável: as outras mantêm a ordem.
    screens.sort_by_key(|s| !s.primary);
    screens
}

/// Ponto `local` de uma superfície que começa em `surface_origin` (global), nas
/// coordenadas da superfície principal (que começa em `main_origin`). Pode ficar fora
/// dos limites da principal (o ponteiro está noutra tela).
pub fn to_main(local: (f32, f32), surface_origin: (f32, f32), main_origin: (f32, f32)) -> (f32, f32) {
    (local.0 + surface_origin.0 - main_origin.0, local.1 + surface_origin.1 - main_origin.1)
}

/// Lê a resposta do comando `outputs` do canal de controle: nome, escala e se é a
/// principal, só das telas ligadas. Linha: `NOME\tdescrição\tLxA@Hz\tX,Y\tescala\t
/// ligada|desligada\tprincipal|-\tmodos`.
pub fn parse_outputs(lines: &[String]) -> Vec<(String, f32, bool)> {
    lines
        .iter()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            if fields.len() < 7 || fields[5] != "ligada" {
                return None;
            }
            let scale = fields[4].parse::<f32>().ok().filter(|s| *s > 0.0).unwrap_or(1.0);
            Some((fields[0].to_string(), scale, fields[6] == "principal"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tela(name: &str, origin: (f32, f32), size: (f32, f32), primary: bool) -> Screen {
        Screen { name: name.into(), origin, size, scale: 1.0, primary }
    }

    fn duas() -> Vec<Screen> {
        vec![tela("HEADLESS-1", (0.0, 0.0), (1366.0, 768.0), true), tela("HEADLESS-2", (1366.0, 0.0), (1920.0, 1080.0), false)]
    }

    #[test]
    fn local_e_global() {
        let b = &duas()[1];
        assert_eq!(b.to_local((1600.0, 300.0)), (234.0, 300.0));
        assert_eq!(b.to_global((234.0, 300.0)), (1600.0, 300.0));
        assert_eq!(b.rect(), (1366.0, 0.0, 1920.0, 1080.0));
        assert!(b.contains((1366.0, 0.0)));
        assert!(!b.contains((1365.9, 10.0)));
        assert!(!b.contains((3286.0, 10.0)));
    }

    #[test]
    fn para_a_principal() {
        // Clique em (100, 50) na segunda tela: na principal (em (0, 0)) é (1466, 50).
        assert_eq!(to_main((100.0, 50.0), (1366.0, 0.0), (0.0, 0.0)), (1466.0, 50.0));
        // Pega implícita: o arrasto começou na principal (à direita) e o ponteiro está
        // na outra; a superfície da principal recebe pontos negativos.
        assert_eq!(to_main((-50.0, 300.0), (1366.0, 0.0), (1366.0, 0.0)), (-50.0, 300.0));
        // Ponto local (−50, 300) da tela 2 é o global (1316, 300).
        assert_eq!(duas()[1].to_global((-50.0, 300.0)), (1316.0, 300.0));
        // Principal à direita da outra.
        assert_eq!(to_main((10.0, 10.0), (0.0, 0.0), (1920.0, 0.0)), (-1910.0, 10.0));
    }

    #[test]
    fn tela_sob_o_ponto() {
        let telas = duas();
        assert_eq!(screen_at(&telas, (1500.0, 300.0)), Some(1));
        assert_eq!(screen_at(&telas, (10.0, 10.0)), Some(0));
        // Buraco embaixo da tela menor.
        assert_eq!(screen_at(&telas, (100.0, 900.0)), None);
    }

    #[test]
    fn uniao_das_telas() {
        assert_eq!(union(&duas()), (0.0, 0.0, 3286.0, 1080.0));
        assert_eq!(union(&[]), (0.0, 0.0, 0.0, 0.0));
        // Tela à esquerda, com origem negativa e mais alta.
        let telas = vec![tela("A", (0.0, 0.0), (1366.0, 768.0), true), tela("B", (-1280.0, -100.0), (1280.0, 1024.0), false)];
        assert_eq!(union(&telas), (-1280.0, -100.0, 2646.0, 1024.0));
    }

    #[test]
    fn espelhadas_viram_uma() {
        // A principal chega depois da cópia: é ela que fica.
        let telas = vec![
            tela("HEADLESS-2", (0.0, 0.0), (1366.0, 769.0), false),
            tela("HEADLESS-1", (0.0, 0.0), (1366.0, 768.0), true),
            tela("HEADLESS-3", (1366.0, 0.0), (800.0, 600.0), false),
        ];
        let ficou = dedupe_mirror(telas);
        assert_eq!(ficou.len(), 2);
        assert_eq!(ficou[0].name, "HEADLESS-1");
        assert_eq!(ficou[1].name, "HEADLESS-3");
        // Sem espelho, nada muda.
        assert_eq!(dedupe_mirror(duas()), duas());
    }

    #[test]
    fn principal_primeiro() {
        let telas = vec![tela("A", (0.0, 0.0), (800.0, 600.0), false), tela("B", (800.0, 0.0), (800.0, 600.0), true), tela("C", (1600.0, 0.0), (800.0, 600.0), false)];
        let nomes: Vec<String> = primary_first(telas).into_iter().map(|s| s.name).collect();
        assert_eq!(nomes, ["B", "A", "C"]);
        // Nenhuma marcada: a primeira vira a principal.
        let sem = primary_first(vec![tela("A", (0.0, 0.0), (800.0, 600.0), false), tela("B", (800.0, 0.0), (800.0, 600.0), false)]);
        assert!(sem[0].primary && sem[0].name == "A" && !sem[1].primary);
    }

    #[test]
    fn superficie_principal_primeiro() {
        // A principal do desktop mudou para HEADLESS-2 com o app aberto: a camada
        // principal continua em HEADLESS-1, que segue em primeiro.
        let mut telas = duas();
        telas[0].primary = false;
        telas[1].primary = true;
        let ficou = main_first(telas.clone(), "HEADLESS-1").unwrap();
        assert_eq!(ficou[0].name, "HEADLESS-1");
        assert_eq!(ficou[1].name, "HEADLESS-2");
        // As marcas de principal não mudam.
        assert!(!ficou[0].primary && ficou[1].primary);
        // A tela da camada principal saiu: sem lista.
        assert_eq!(main_first(vec![telas[1].clone()], "HEADLESS-1"), None);
        // Espelhadas: fica a da superfície principal, mesmo sem ser a principal.
        let espelho = vec![tela("HEADLESS-2", (0.0, 0.0), (1366.0, 768.0), true), tela("HEADLESS-1", (0.0, 0.0), (1366.0, 769.0), false)];
        let ficou = main_first(espelho, "HEADLESS-1").unwrap();
        assert_eq!(ficou.len(), 1);
        assert_eq!(ficou[0].name, "HEADLESS-1");
    }

    #[test]
    fn le_o_comando_outputs() {
        let linhas = vec![
            "HEADLESS-1\t-\t1366x768@60\t0,0\t1\tligada\t-\t1366x768@60".to_string(),
            "HEADLESS-2\tMonitor X\t1920x1080@60\t1366,0\t1.25\tligada\tprincipal\t1920x1080@60".to_string(),
            "HDMI-A-1\t-\t1920x1080@60\t0,0\t1\tdesligada\t-\t1920x1080@60".to_string(),
            "uso: outputs [apply]".to_string(),
        ];
        assert_eq!(parse_outputs(&linhas), vec![("HEADLESS-1".to_string(), 1.0, false), ("HEADLESS-2".to_string(), 1.25, true)]);
    }
}
