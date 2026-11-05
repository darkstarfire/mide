//! Legendas do player de vídeo: leitura de SubRip (`.srt`) e WebVTT (`.vtt`), escrita
//! do zero, e a busca da legenda do instante. As legendas em texto de dentro do vídeo
//! (SubRip, ASS/SSA, mov_text, WebVTT) são convertidas para SubRip pelo `ffmpeg`, que já
//! é dependência do mide; legendas em imagem (PGS, DVD) não entram.
//!
//! Para quem vem do Go: `Cue` é uma struct simples; a lista fica ordenada pelo início e a
//! busca é um `sort.Search` (`partition_point`).

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// Uma fala: começo e fim em segundos e o texto (linhas separadas por `\n`).
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

/// Lê SubRip ou WebVTT (o formato é reconhecido pelo conteúdo). Blocos sem tempo
/// válido são pulados; o resultado sai ordenado pelo início.
pub fn parse(text: &str) -> Vec<Cue> {
    let text = text.trim_start_matches('\u{feff}').replace("\r\n", "\n").replace('\r', "\n");
    let mut cues = Vec::new();
    for block in text.split("\n\n") {
        let mut lines = block.lines().filter(|l| !l.trim().is_empty()).peekable();
        // Número do SubRip, ou o identificador do WebVTT, antes da linha de tempo.
        let Some(timing) = lines.by_ref().find(|l| l.contains("-->")) else {
            continue;
        };
        let Some((start, end)) = parse_timing(timing) else {
            continue;
        };
        let body: Vec<String> = lines.map(clean_line).filter(|l| !l.is_empty()).collect();
        if body.is_empty() || end <= start {
            continue;
        }
        cues.push(Cue { start, end, text: body.join("\n") });
    }
    cues.sort_by(|a, b| a.start.total_cmp(&b.start));
    cues
}

/// "00:01:02,500 --> 00:01:04,000" (e o WebVTT com ponto, sem horas e com ajustes
/// de posição depois do tempo final).
fn parse_timing(line: &str) -> Option<(f64, f64)> {
    let (a, b) = line.split_once("-->")?;
    let b = b.split_whitespace().next()?;
    Some((parse_time(a.trim())?, parse_time(b.trim())?))
}

/// "01:02:03,456", "02:03.456" ou "3.5" em segundos.
fn parse_time(s: &str) -> Option<f64> {
    let s = s.replace(',', ".");
    let parts: Vec<&str> = s.split(':').collect();
    let secs: f64 = parts.last()?.parse().ok()?;
    let mut total = secs;
    let mut mult = 60.0;
    for p in parts.iter().rev().skip(1) {
        total += p.trim().parse::<f64>().ok()? * mult;
        mult *= 60.0;
    }
    (total >= 0.0).then_some(total)
}

/// Tira as marcações de estilo (`<i>`, `<font ...>`, `{\an8}` do ASS convertido) e
/// troca as entidades comuns do WebVTT.
fn clean_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut skip_tag = false;
    let mut skip_brace = false;
    for c in line.chars() {
        match c {
            '<' => skip_tag = true,
            '>' if skip_tag => skip_tag = false,
            '{' => skip_brace = true,
            '}' if skip_brace => skip_brace = false,
            _ if skip_tag || skip_brace => {}
            _ => out.push(c),
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ").replace("\\N", "\n").trim().to_string()
}

/// O texto da fala no instante `t`, se houver.
pub fn cue_at(cues: &[Cue], t: f64) -> Option<&Cue> {
    // A última que começou até `t`; falas sobrepostas são raras e a mais nova vale.
    let i = cues.partition_point(|c| c.start <= t);
    cues[..i].iter().rev().take(4).find(|c| t < c.end)
}

/// Arquivos de legenda ao lado do vídeo: `filme.srt`, `filme.pt-BR.srt`, `filme.vtt`...
/// Devolve o caminho e o rótulo (o que vem entre o nome do vídeo e a extensão, ou o
/// nome do arquivo).
pub fn sidecar_files(video: &Path) -> Vec<(PathBuf, String)> {
    let (Some(dir), Some(stem)) = (video.parent(), video.file_stem().map(|s| s.to_string_lossy().into_owned())) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(PathBuf, String)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("srt") || e.eq_ignore_ascii_case("vtt")))
        .filter_map(|p| {
            let name = p.file_stem()?.to_string_lossy().into_owned();
            let rest = name.strip_prefix(&stem)?;
            let label = rest.trim_start_matches(['.', '_', '-', ' ']);
            let label = if label.is_empty() { p.file_name()?.to_string_lossy().into_owned() } else { label.to_string() };
            Some((p, label))
        })
        .collect();
    found.sort_by(|a, b| a.1.cmp(&b.1));
    found
}

/// Formatos de legenda em texto que o `ffmpeg` converte para SubRip.
pub fn is_text_codec(codec: &str) -> bool {
    matches!(codec, "subrip" | "srt" | "ass" | "ssa" | "mov_text" | "webvtt" | "text")
}

/// Extrai a legenda embutida número `n` (entre as legendas do vídeo) como SubRip.
/// Demora conforme o tamanho do arquivo: chame fora da thread da interface.
pub fn extract_embedded(video: &Path, n: usize) -> Option<Vec<Cue>> {
    let out = Command::new("ffmpeg")
        .args(["-nostdin", "-loglevel", "error", "-i"])
        .arg(video)
        .args(["-map", &format!("0:s:{n}"), "-f", "srt", "-"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    out.status.success().then(|| parse(&String::from_utf8_lossy(&out.stdout)))
}

/// Lê um arquivo de legenda (UTF-8; com outra codificação, os caracteres que não
/// casam viram �, sem perder o resto).
pub fn load_file(path: &Path) -> Option<Vec<Cue>> {
    let bytes = std::fs::read(path).ok()?;
    let text = match String::from_utf8(bytes) {
        Ok(t) => t,
        // Legendas antigas em Latin-1 (comuns no Brasil): cada byte é um caractere.
        Err(e) => e.into_bytes().iter().map(|&b| b as char).collect(),
    };
    Some(parse(&text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn le_subrip() {
        let srt = "1\r\n00:00:01,000 --> 00:00:02,500\r\n<i>Olá,</i> mundo\r\nsegunda linha\r\n\r\n2\r\n00:00:03,000 --> 00:00:04,000\r\n{\\an8}Em cima\r\n";
        let cues = parse(srt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0], Cue { start: 1.0, end: 2.5, text: "Olá, mundo\nsegunda linha".into() });
        assert_eq!(cues[1].text, "Em cima");
        assert_eq!(cue_at(&cues, 2.0).map(|c| c.text.as_str()), Some("Olá, mundo\nsegunda linha"));
        assert!(cue_at(&cues, 2.7).is_none());
        assert_eq!(cue_at(&cues, 3.5).map(|c| c.start), Some(3.0));
        assert!(cue_at(&cues, 0.5).is_none());
    }

    #[test]
    fn le_webvtt() {
        let vtt = "WEBVTT\n\nNOTE comentário\n\nid1\n01:02.000 --> 01:03.500 align:start\nTom &amp; Jerry\n\n00:00:10.000 --> 00:00:11.000\n<v Ana>Oi</v>\n";
        let cues = parse(vtt);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0], Cue { start: 10.0, end: 11.0, text: "Oi".into() });
        assert_eq!(cues[1], Cue { start: 62.0, end: 63.5, text: "Tom & Jerry".into() });
    }

    #[test]
    fn acha_legendas_ao_lado_do_video() {
        let dir = std::env::temp_dir().join(format!("mide-subs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["filme.mp4", "filme.srt", "filme.pt-BR.srt", "filme.en.vtt", "outro.srt"] {
            std::fs::write(dir.join(f), "").unwrap();
        }
        let labels: Vec<String> = sidecar_files(&dir.join("filme.mp4")).into_iter().map(|(_, l)| l).collect();
        assert_eq!(labels, vec!["en", "filme.srt", "pt-BR"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn latin1_nao_perde_o_arquivo() {
        let dir = std::env::temp_dir().join(format!("mide-subs-l1-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("a.srt");
        std::fs::write(&file, b"1\n00:00:01,000 --> 00:00:02,000\nA\xe7\xe3o\n").unwrap();
        assert_eq!(load_file(&file).unwrap()[0].text, "Ação");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
